//! Local API server on 127.0.0.1 (see usbipb_core::api).

use crate::engine::Engine;
use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use usbipb_core::api::{Request, Response};

fn result(r: Result<String>) -> Response {
    match r {
        Ok(message) => Response::Ok { message },
        Err(e) => Response::Error { message: format!("{e:#}") },
    }
}

pub fn spawn(engine: Arc<Engine>) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", engine.config.api_port)).with_context(|| format!("bind local API port {}", engine.config.api_port))?;
    listener.set_nonblocking(true)?;
    log::info!("local API on 127.0.0.1:{}", engine.config.api_port);
    std::thread::spawn(move || {
        while !engine.stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut line = String::new();
                    let mut reader = BufReader::new(match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => continue,
                    });
                    if reader.read_line(&mut line).is_err() {
                        continue;
                    }
                    let response = match serde_json::from_str::<Request>(&line) {
                        Ok(Request::Status) => Response::Status(engine.status()),
                        Ok(Request::Attach { index }) => result(engine.set_wanted(index, true)),
                        Ok(Request::Detach { index }) => result(engine.set_wanted(index, false)),
                        Ok(Request::Share { device, share }) => result(engine.set_shared(device, share)),
                        Ok(Request::Use { host, port, device, attach }) => result(engine.use_device(&host, port, device, attach)),
                        Ok(Request::Give { peer, device }) => result(engine.give(&peer, device)),
                        Ok(Request::TakeBack { device }) => result(engine.take_back(device)),
                        Ok(Request::AddPeer { address }) => result(crate::peers::add_peer(&engine, &address)),
                        Ok(Request::RemovePeer { address }) => result(crate::peers::remove_peer(&engine, &address)),
                        Ok(Request::ReloadAudio) => result(engine.reload_audio()),
                        Ok(Request::Pair { address }) => result(crate::security::start_pairing(&engine, &address)),
                        Ok(Request::PairAnswer { id, code }) => result(engine.security.answer(id, code)),
                        Ok(Request::Unpair { host_id }) => result(crate::security::unpair(&engine, &host_id)),
                        Ok(Request::Lock { device, lock }) => result(engine.set_locked(device, lock)),
                        Err(e) => Response::Error { message: format!("bad request: {e}") },
                    };
                    let mut out = serde_json::to_string(&response).unwrap_or_default();
                    out.push('\n');
                    let _ = (&stream).write_all(out.as_bytes());
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                Err(e) => {
                    log::error!("local API: {e}");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    });
    Ok(())
}
