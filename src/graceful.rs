use std::{
    env,
    fs::OpenOptions,
    io::{Error, ErrorKind, Write, stderr},
    net::TcpListener as StdTcpListener,
    os::unix::{
        io::{AsRawFd, FromRawFd, RawFd},
        net::UnixListener as StdUnixListener,
    },
    path::Path,
    process,
    process::Command,
    thread,
    time::Duration,
};

use close_fds::set_fds_cloexec;
use log::*;
use signal_hook::consts as signal_const;
use signal_hook::iterator::Signals;
use tokio::net::{TcpListener as TokioTcpListener, UnixListener as TokioUnixListener};

use super::net::{UnifyAddr, UnifyListener};

pub fn write_pid_file(run_dir: &str, prog_name: &str) -> std::io::Result<()> {
    let pid_file_path = Path::new(run_dir).join(format!("{}.pid", prog_name));
    let mut f = OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(pid_file_path.to_str().unwrap())?;
    let pid = process::id();
    f.write_all(pid.to_string().as_bytes())?;
    f.sync_data()?;
    Ok(())
}

/// Call after program is fork. Use a seperate empty file to notify parent that child has started up.
fn write_child_pid_file(run_dir: &str, prog_name: &str) -> std::io::Result<()> {
    let pid_file_path = Path::new(run_dir).join(format!("{}_{}", prog_name, process::id()));
    std::fs::File::create(pid_file_path.to_str().unwrap())?;
    Ok(())
}

pub const RESTART_TIMEOUT: usize = 200;

/// For service that support graceful restart.
/// All socket listener managed by `Graceful` is inherited to the new program after restart.
/// New connection goes to new program. Old connections continue to be served at old program.
pub struct Graceful {
    run_dir: String,
    prog_name: String,
    listen_fds: Vec<String>,
    recovered: bool,
    recover_listen_fds: Vec<RawFd>,
}

impl Graceful {
    #[inline]
    pub fn new(run_dir: String, prog_name: String) -> Self {
        let mut graceful = Graceful {
            run_dir,
            prog_name,
            listen_fds: Vec::new(),
            recovered: false,
            recover_listen_fds: Vec::new(),
        };
        graceful.check_recover();
        return graceful;
    }

    fn check_recover(&mut self) {
        if let Some(env_recover_fds_str) = env::var_os("_GRACEFUL_RESTART") {
            let fds: Vec<&str> = env_recover_fds_str.to_str().unwrap().split(',').collect();
            for fd_str in fds {
                let fd = fd_str.parse::<RawFd>().unwrap();
                self.recover_listen_fds.push(fd);
            }
            self.recovered = true;
        }
    }

    fn restart(&mut self) -> std::io::Result<()> {
        let mut args = std::env::args_os();
        let arg0 = args.next().unwrap();
        let mut cmd = Command::new(arg0);
        while let Some(arg) = args.next() {
            cmd.arg(arg);
        }

        let mut preserved_fds = Vec::new();
        for fd_str in &self.listen_fds {
            let fd = fd_str.parse::<RawFd>().unwrap();
            preserved_fds.push(fd);
        }

        let listen_fds_str = self.listen_fds.join(",");
        cmd.env("_GRACEFUL_RESTART", &listen_fds_str);
        // set FD_CLOEXEC to parnet's fds except listening fds.
        // listening fds need to be inherited to support graceful restart
        preserved_fds.sort();
        let min_fd = stderr().as_raw_fd() + 1;
        set_fds_cloexec(min_fd, &preserved_fds);

        // parent fork child process failed, parent do not exit
        let mut child = cmd.spawn()?;
        let child_pid = child.id();
        let child_pid_file_path =
            Path::new(&self.run_dir).join(format!("{}_{}", &self.prog_name, child_pid));
        for _ in 0..RESTART_TIMEOUT {
            match std::fs::File::open(&child_pid_file_path) {
                Ok(_) => {
                    // parent remove parent_child_sync_file failed but graceful restart success, parent still exit
                    let _ = std::fs::remove_file(&child_pid_file_path);
                    return Ok(());
                }
                Err(_) => {
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }
        match child.try_wait() {
            Ok(Some(_)) => {
                // graceful restart failed child is exited, parent do not exit
                return Err(Error::new(ErrorKind::Other, "graceful restart failed, child exited"));
            }
            Ok(None) => {
                // graceful restart triggered but child is not ready, parent still exit
                return Ok(());
            }
            Err(e) => {
                return Err(e);
            }
        }
    }

    /// Initiate socket listener which supports graceful restart.
    /// We assume the program does not change addrs, always call with the same order.
    pub fn new_unify_listener(
        &mut self, rt: &tokio::runtime::Runtime, unify_addr: &UnifyAddr,
    ) -> std::io::Result<UnifyListener> {
        // XXX What if order wrong?
        if self.recover_listen_fds.len() > 0 {
            let raw_fd = self.recover_listen_fds.remove(0);
            match unify_addr {
                UnifyAddr::Socket(_) => {
                    let std_listener = unsafe { StdTcpListener::from_raw_fd(raw_fd) };
                    if let Err(e) = std_listener.set_nonblocking(true) {
                        error!("cannot set non-blocking from unix fd {}", raw_fd);
                        return Err(e);
                    }
                    let tokio_listener = rt
                        .block_on(async move { TokioTcpListener::from_std(std_listener).unwrap() });
                    self.listen_fds.push(raw_fd.to_string());
                    return Ok(UnifyListener::Tcp(tokio_listener));
                }
                UnifyAddr::Path(_) => {
                    let std_listener = unsafe { StdUnixListener::from_raw_fd(raw_fd) };
                    if let Err(e) = std_listener.set_nonblocking(true) {
                        error!("cannot set non-blocking from unix fd {}", raw_fd);
                        return Err(e);
                    }
                    let tokio_listener =
                        rt.block_on(
                            async move { TokioUnixListener::from_std(std_listener).unwrap() },
                        );
                    self.listen_fds.push(raw_fd.to_string());
                    return Ok(UnifyListener::Unix(tokio_listener));
                }
            }
        } else {
            let unify_listener =
                rt.block_on(async move { UnifyListener::bind(unify_addr).await })?;
            let raw_fd: RawFd;
            match unify_listener {
                UnifyListener::Tcp(ref tcp_listener) => {
                    raw_fd = tcp_listener.as_raw_fd();
                }
                UnifyListener::Unix(ref unix_listener) => {
                    raw_fd = unix_listener.as_raw_fd();
                }
            }
            unsafe {
                libc::fcntl(raw_fd, libc::F_SETFD, 0);
            }
            self.listen_fds.push(raw_fd.to_string());
            return Ok(unify_listener);
        }
    }

    pub fn ready<F: FnOnce()>(
        &mut self, exit_closure: F, can_graceful_restart: bool,
    ) -> std::io::Result<()> {
        // write process_pid_file failed, process start failed and exit
        write_pid_file(&self.run_dir, &self.prog_name)?;
        if self.recovered {
            // child creat parent_child_sync_file failed, child still continue run
            let _ = write_child_pid_file(&self.run_dir, &self.prog_name);
        }
        let mut signals = Signals::new(&[
            signal_const::SIGHUP,
            signal_const::SIGTERM,
            signal_const::SIGINT,
            signal_const::SIGQUIT,
        ])
        .unwrap();
        for signal in signals.forever() {
            if can_graceful_restart && signal == signal_const::SIGHUP {
                // graceful restart
                match self.restart() {
                    Ok(_) => break,
                    Err(_) => continue,
                }
            }
            // graceful exit
            break;
        }
        exit_closure();
        return Ok(());
    }
}
