use crossbeam_channel::{Receiver, unbounded};
use std::io::{ErrorKind, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const INSTANCE_ADDRESS: &str = "127.0.0.1:47633";
const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
const INSTANCE_ACK: &[u8; 2] = b"OK";

pub enum InstanceMode {
    Primary(SingleInstance),
    Secondary,
}

pub struct SingleInstance {
    initial_paths: Vec<PathBuf>,
    receiver: Receiver<Vec<PathBuf>>,
}

impl SingleInstance {
    pub fn initial_paths(&self) -> &[PathBuf] {
        &self.initial_paths
    }

    pub fn receiver(&self) -> Receiver<Vec<PathBuf>> {
        self.receiver.clone()
    }
}

pub fn initialize(initial_paths: Vec<PathBuf>) -> InstanceMode {
    match TcpListener::bind(INSTANCE_ADDRESS) {
        Ok(listener) => {
            let (sender, receiver) = unbounded();
            thread::spawn(move || {
                for connection in listener.incoming() {
                    let Ok(mut stream) = connection else {
                        continue;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    let mut data = Vec::new();
                    {
                        let mut limited_stream = (&mut stream).take(MAX_MESSAGE_SIZE as u64);
                        if limited_stream.read_to_end(&mut data).is_err() {
                            continue;
                        }
                    }
                    let Ok(paths) = serde_json::from_slice::<Vec<String>>(&data) else {
                        continue;
                    };
                    let paths = paths.into_iter().map(PathBuf::from).collect::<Vec<_>>();
                    if sender.send(paths).is_ok() {
                        let _ = stream.write_all(INSTANCE_ACK);
                        let _ = stream.flush();
                    }
                }
            });
            InstanceMode::Primary(SingleInstance {
                initial_paths,
                receiver,
            })
        }
        Err(error) if error.kind() == ErrorKind::AddrInUse => {
            if forward_to_primary(&initial_paths) {
                InstanceMode::Secondary
            } else {
                let (_, receiver) = unbounded();
                InstanceMode::Primary(SingleInstance {
                    initial_paths,
                    receiver,
                })
            }
        }
        Err(_) => {
            let (_, receiver) = unbounded();
            InstanceMode::Primary(SingleInstance {
                initial_paths,
                receiver,
            })
        }
    }
}

fn forward_to_primary(paths: &[PathBuf]) -> bool {
    let payload = paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let Ok(payload) = serde_json::to_vec(&payload) else {
        return false;
    };

    for _ in 0..20 {
        if let Ok(mut stream) = TcpStream::connect(INSTANCE_ADDRESS) {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
            let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
            if stream.write_all(&payload).is_ok()
                && stream.flush().is_ok()
                && stream.shutdown(Shutdown::Write).is_ok()
            {
                let mut ack = [0u8; INSTANCE_ACK.len()];
                if stream.read_exact(&mut ack).is_ok() && &ack == INSTANCE_ACK {
                    return true;
                }
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}
