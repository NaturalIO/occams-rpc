use rand::{Rng, RngCore};
use razor_rpc_core::io::*;
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};

#[derive(Debug)]
enum MockReadBehavior {
    Chunked(Vec<Vec<u8>>),
    Randomized { data: Vec<u8>, pos: usize },
}

// A mock stream for read operations only
#[derive(Debug)]
struct MockReadStream {
    read_behavior: MockReadBehavior,
}

impl MockReadStream {
    fn new_chunked_reader(chunks: Vec<Vec<u8>>) -> Self {
        Self { read_behavior: MockReadBehavior::Chunked(chunks) }
    }

    fn new_chunked_reader_deterministic(chunks: Vec<Vec<u8>>) -> Self {
        // For deterministic reads, we just use the same chunked reader
        Self { read_behavior: MockReadBehavior::Chunked(chunks) }
    }

    fn new_randomized_reader(data: Vec<u8>) -> Self {
        Self { read_behavior: MockReadBehavior::Randomized { data, pos: 0 } }
    }
}

impl AsyncRead for MockReadStream {
    fn read(&mut self, buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send {
        async move {
            match &mut self.read_behavior {
                MockReadBehavior::Chunked(chunks) => {
                    if chunks.is_empty() {
                        return Ok(0);
                    }
                    let chunk = chunks.remove(0);
                    let n = std::cmp::min(buf.len(), chunk.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    Ok(n)
                }
                MockReadBehavior::Randomized { data, pos } => {
                    if *pos >= data.len() {
                        return Ok(0); // True EOF
                    }
                    let mut rng = rand::thread_rng();
                    let remaining = data.len() - *pos;
                    let max_read = std::cmp::min(buf.len(), remaining);
                    if max_read == 0 {
                        return Ok(0);
                    }
                    let read_size = rng.gen_range(1..=max_read);

                    buf[..read_size].copy_from_slice(&data[*pos..*pos + read_size]);
                    *pos += read_size;
                    Ok(read_size)
                }
            }
        }
    }
}

// A mock stream for write operations with buffering support
#[derive(Debug)]
struct MockWriteStream {
    write_buffer: Arc<Mutex<Vec<u8>>>,
    deterministic: bool, // Flag to control deterministic behavior for writes
}

impl MockWriteStream {
    fn new(write_buffer: Arc<Mutex<Vec<u8>>>, deterministic: bool) -> Self {
        Self { write_buffer, deterministic }
    }
}

impl AsyncRead for MockWriteStream {
    fn read(&mut self, _buf: &mut [u8]) -> impl Future<Output = io::Result<usize>> + Send {
        async move {
            // For write-only stream, always return EOF
            Ok(0)
        }
    }
}

impl AsyncWrite for MockWriteStream {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> + Send {
        async move {
            if buf.is_empty() {
                return Ok(0);
            }

            let n = if self.deterministic {
                // In deterministic mode, always write the full buffer
                buf.len()
            } else {
                // In random mode, sometimes do short writes
                let mut rng = rand::thread_rng();
                // Using a 50% chance for short writes to make it more likely to occur in tests.
                if rng.gen_bool(0.5) { rng.gen_range(1..=buf.len()) } else { buf.len() }
            };

            self.write_buffer.lock().unwrap().extend_from_slice(&buf[..n]);
            Ok(n)
        }
    }
}

// ==================== DETERMINISTIC TESTS ====================

#[tokio::test]
async fn test_async_read_exact_fixed_chunks() {
    // Use fixed, deterministic values
    let data_size = 2048;
    let mut source_data = vec![0u8; data_size];
    // Fill with deterministic pattern
    for i in 0..data_size {
        source_data[i] = (i % 256) as u8;
    }

    // Create fixed chunks
    let chunks = vec![
        source_data[0..512].to_vec(),
        source_data[512..1024].to_vec(),
        source_data[1024..1536].to_vec(),
        source_data[1536..2048].to_vec(),
    ];

    let mut read_stream = MockReadStream::new_chunked_reader_deterministic(chunks);

    let mut out = vec![0u8; data_size];
    read_stream.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_read_bypass_fixed() {
    // Use fixed, deterministic values
    let data_size = 300; // > 256 to test bypass
    let mut source_data = vec![0u8; data_size];
    // Fill with deterministic pattern
    for i in 0..data_size {
        source_data[i] = (i % 256) as u8;
    }

    let mut read_stream =
        MockReadStream::new_chunked_reader_deterministic(vec![source_data.clone()]);

    let mut out = vec![0u8; data_size];
    read_stream.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_read_multiple_reads_fixed() {
    // Use fixed, deterministic values
    let chunk1_data = vec![1u8; 100];
    let chunk2_data = vec![2u8; 100];

    let chunks = vec![chunk1_data.clone(), chunk2_data.clone()];
    let mut read_stream = MockReadStream::new_chunked_reader_deterministic(chunks);

    let mut out1 = vec![0u8; 100];
    read_stream.read_exact(&mut out1).await.unwrap();
    assert_eq!(out1, chunk1_data);

    let mut out2 = vec![0u8; 100];
    read_stream.read_exact(&mut out2).await.unwrap();
    assert_eq!(out2, chunk2_data);
}

#[tokio::test]
async fn test_async_write_all_buffering_deterministic() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockWriteStream::new(data_handle.clone(), true); // deterministic = true
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"hello").await.unwrap();
    {
        assert!(data_handle.lock().unwrap().is_empty()); // buffered
    }
    writer.write_all(b" wo").await.unwrap(); // total 5+3=8, should not flush yet
    {
        assert!(data_handle.lock().unwrap().is_empty()); // still buffered, pos is 8
    }

    writer.write_all(b"rld").await.unwrap(); // overflows buffer
    // "hello wo" should be flushed.
    {
        assert_eq!(*data_handle.lock().unwrap(), b"hello wo");
    }
    // "rld" is in the buffer

    writer.flush().await.unwrap();
    {
        assert_eq!(*data_handle.lock().unwrap(), b"hello world");
    }
}

#[tokio::test]
async fn test_async_write_bypass_deterministic() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockWriteStream::new(data_handle.clone(), true); // deterministic = true
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"abc").await.unwrap();
    {
        assert!(data_handle.lock().unwrap().is_empty()); // buffered
    }
    // This write is larger than the buffer, it should bypass it.
    writer.write_all(b"this is a long line").await.unwrap();
    // The buffer "abc" should be flushed first.
    // Then "this is a long line" is written directly.
    {
        assert_eq!(*data_handle.lock().unwrap(), b"abcthis is a long line");
    }

    writer.flush().await.unwrap();
    {
        assert_eq!(*data_handle.lock().unwrap(), b"abcthis is a long line");
    }
}

// ==================== RANDOMIZED TESTS ====================

#[tokio::test]
async fn test_async_read_exact_random_chunks() {
    let mut rng = rand::thread_rng();
    let data_size = rng.gen_range(1024..4096);
    let mut source_data = vec![0u8; data_size];
    rng.fill_bytes(&mut source_data);

    let mut chunks = Vec::new();
    let mut remaining_data = &source_data[..];
    while !remaining_data.is_empty() {
        let chunk_size = rng.gen_range(1..128).min(remaining_data.len());
        chunks.push(remaining_data[..chunk_size].to_vec());
        remaining_data = &remaining_data[chunk_size..];
    }

    let mut read_stream = MockReadStream::new_chunked_reader(chunks);

    let mut out = vec![0u8; data_size];
    read_stream.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_read_bypass_random() {
    let mut rng = rand::thread_rng();
    let data_size = rng.gen_range(257..512);
    let mut source_data = vec![0u8; data_size];
    rng.fill_bytes(&mut source_data);

    let mut read_stream = MockReadStream::new_chunked_reader(vec![source_data.clone()]);

    let mut out = vec![0u8; data_size];
    read_stream.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_read_multiple_reads_random() {
    let mut rng = rand::thread_rng();
    let chunk1_size = rng.gen_range(64..128);
    let mut chunk1_data = vec![0u8; chunk1_size];
    rng.fill_bytes(&mut chunk1_data);

    let chunk2_size = rng.gen_range(64..128);
    let mut chunk2_data = vec![0u8; chunk2_size];
    rng.fill_bytes(&mut chunk2_data);

    let chunks = vec![chunk1_data.clone(), chunk2_data.clone()];
    let mut read_stream = MockReadStream::new_chunked_reader(chunks);

    let mut out1 = vec![0u8; chunk1_size];
    read_stream.read_exact(&mut out1).await.unwrap();
    assert_eq!(out1, chunk1_data);

    let mut out2 = vec![0u8; chunk2_size];
    read_stream.read_exact(&mut out2).await.unwrap();
    assert_eq!(out2, chunk2_data);
}

#[tokio::test]
async fn test_random_read_sizes_and_returns() {
    let mut rng = rand::thread_rng();
    let data_size = rng.gen_range(8192..16384);
    let mut source_data = vec![0u8; data_size];
    rng.fill_bytes(&mut source_data);

    let mut read_stream = MockReadStream::new_randomized_reader(source_data.clone());

    let mut result_data = Vec::with_capacity(data_size);
    while result_data.len() < data_size {
        let read_size = rng.gen_range(1..=512);
        let mut temp_buf = vec![0u8; read_size];
        match read_stream.read(&mut temp_buf).await {
            Ok(0) => break, // EOF
            Ok(n) => {
                result_data.extend_from_slice(&temp_buf[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => panic!("Read failed: {}", e),
        }
    }

    assert_eq!(result_data.len(), data_size);
    assert_eq!(result_data, source_data);
}

#[tokio::test]
async fn test_async_write_all_buffering_random() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockWriteStream::new(data_handle.clone(), false); // deterministic = false
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"hello").await.unwrap();
    {
        assert!(data_handle.lock().unwrap().is_empty()); // buffered
    }
    writer.write_all(b" wo").await.unwrap(); // total 5+3=8, should not flush yet
    {
        assert!(data_handle.lock().unwrap().is_empty()); // still buffered, pos is 8
    }

    writer.write_all(b"rld").await.unwrap(); // overflows buffer
    // "hello wo" should be flushed.
    {
        assert_eq!(*data_handle.lock().unwrap(), b"hello wo");
        // "rld" is in the buffer
    }
    writer.flush().await.unwrap();
    {
        assert_eq!(*data_handle.lock().unwrap(), b"hello world");
    }
}

#[tokio::test]
async fn test_async_write_bypass_random() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockWriteStream::new(data_handle.clone(), false); // deterministic = false
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"abc").await.unwrap();
    {
        assert!(data_handle.lock().unwrap().is_empty()); // buffered
    }
    // This write is larger than the buffer, it should bypass it.
    writer.write_all(b"this is a long line").await.unwrap();
    // due to short writes, the behavior cannot be assert
    writer.flush().await.unwrap();
    {
        assert_eq!(*data_handle.lock().unwrap(), b"abcthis is a long line");
    }
}
