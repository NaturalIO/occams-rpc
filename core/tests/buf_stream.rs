use occams_rpc_core::io::*;
use rand::{Rng, RngCore};
use std::future::Future;
use std::io;
use std::sync::{Arc, Mutex};

enum MockReadBehavior {
    Chunked(Vec<Vec<u8>>),
    Randomized { data: Vec<u8>, pos: usize },
}

// A mock stream that can return data in fixed chunks or in random sub-chunks for reading,
// and captures written data.
struct MockStream {
    read_behavior: MockReadBehavior,
    write_buffer: Arc<Mutex<Vec<u8>>>,
}

impl MockStream {
    fn new_chunked_reader(chunks: Vec<Vec<u8>>, write_buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { read_behavior: MockReadBehavior::Chunked(chunks), write_buffer }
    }

    fn new_randomized_reader(data: Vec<u8>, write_buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self { read_behavior: MockReadBehavior::Randomized { data, pos: 0 }, write_buffer }
    }

    fn new_writer(write_buffer: Arc<Mutex<Vec<u8>>>) -> Self {
        Self {
            read_behavior: MockReadBehavior::Chunked(vec![]), // No reading
            write_buffer,
        }
    }
}

impl AsyncRead for MockStream {
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

impl AsyncWrite for MockStream {
    fn write(&mut self, buf: &[u8]) -> impl Future<Output = io::Result<usize>> + Send {
        async move {
            if buf.is_empty() {
                return Ok(0);
            }
            let mut rng = rand::thread_rng();
            // Using a 50% chance for short writes to make it more likely to occur in tests.
            let n = if rng.gen_bool(0.5) { rng.gen_range(1..=buf.len()) } else { buf.len() };

            self.write_buffer.lock().unwrap().extend_from_slice(&buf[..n]);
            Ok(n)
        }
    }
}

#[tokio::test]
async fn test_async_buf_read_exact_random_chunks() {
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

    let mock_stream = MockStream::new_chunked_reader(chunks, Arc::new(Mutex::new(Vec::new())));
    let mut reader = AsyncBufStream::new(mock_stream, 256);

    let mut out = vec![0u8; data_size];
    reader.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_buf_read_bypass_random() {
    let mut rng = rand::thread_rng();
    let data_size = rng.gen_range(257..512);
    let mut source_data = vec![0u8; data_size];
    rng.fill_bytes(&mut source_data);

    let mock_stream =
        MockStream::new_chunked_reader(vec![source_data.clone()], Arc::new(Mutex::new(Vec::new())));
    let mut reader = AsyncBufStream::new(mock_stream, 256);

    let mut out = vec![0u8; data_size];
    reader.read_exact(&mut out).await.unwrap();
    assert_eq!(out, source_data);
}

#[tokio::test]
async fn test_async_buf_read_multiple_reads_random() {
    let mut rng = rand::thread_rng();
    let chunk1_size = rng.gen_range(64..128);
    let mut chunk1_data = vec![0u8; chunk1_size];
    rng.fill_bytes(&mut chunk1_data);

    let chunk2_size = rng.gen_range(64..128);
    let mut chunk2_data = vec![0u8; chunk2_size];
    rng.fill_bytes(&mut chunk2_data);

    let chunks = vec![chunk1_data.clone(), chunk2_data.clone()];
    let mock_stream = MockStream::new_chunked_reader(chunks, Arc::new(Mutex::new(Vec::new())));
    let mut reader = AsyncBufStream::new(mock_stream, 100);

    let mut out1 = vec![0u8; chunk1_size];
    reader.read_exact(&mut out1).await.unwrap();
    assert_eq!(out1, chunk1_data);

    let mut out2 = vec![0u8; chunk2_size];
    reader.read_exact(&mut out2).await.unwrap();
    assert_eq!(out2, chunk2_data);
}

#[tokio::test]
async fn test_random_read_sizes_and_returns() {
    let mut rng = rand::thread_rng();
    let data_size = rng.gen_range(8192..16384);
    let mut source_data = vec![0u8; data_size];
    rng.fill_bytes(&mut source_data);

    let mock_stream =
        MockStream::new_randomized_reader(source_data.clone(), Arc::new(Mutex::new(Vec::new())));
    let mut reader = AsyncBufStream::new(mock_stream, 1024);

    let mut result_data = Vec::with_capacity(data_size);
    while result_data.len() < data_size {
        let read_size = rng.gen_range(1..=512);
        let mut temp_buf = vec![0u8; read_size];
        match reader.read(&mut temp_buf).await {
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
async fn test_async_buf_write_all_buffering() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockStream::new_writer(data_handle.clone());
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"hello").await.unwrap();
    assert!(data_handle.lock().unwrap().is_empty()); // buffered

    writer.write_all(b" wo").await.unwrap(); // total 5+3=8, should not flush yet
    assert!(data_handle.lock().unwrap().is_empty()); // still buffered, pos is 8

    writer.write_all(b"rld").await.unwrap(); // overflows buffer
    // "hello wo" should be flushed.
    assert_eq!(*data_handle.lock().unwrap(), b"hello wo");
    // "rld" is in the buffer

    writer.flush().await.unwrap();
    assert_eq!(*data_handle.lock().unwrap(), b"hello world");
}

#[tokio::test]
async fn test_async_buf_write_bypass() {
    let data_handle = Arc::new(Mutex::new(Vec::new()));
    let mock_stream = MockStream::new_writer(data_handle.clone());
    let mut writer = AsyncBufStream::new(mock_stream, 8);

    writer.write_all(b"abc").await.unwrap();
    assert!(data_handle.lock().unwrap().is_empty()); // buffered

    // This write is larger than the buffer, it should bypass it.
    writer.write_all(b"this is a long line").await.unwrap();
    // The buffer "abc" should be flushed first.
    // Then "this is a long line" is written directly.
    assert_eq!(*data_handle.lock().unwrap(), b"abcthis is a long line");

    writer.flush().await.unwrap();
    assert_eq!(*data_handle.lock().unwrap(), b"abcthis is a long line");
}
