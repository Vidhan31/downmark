//! Inter-process communication for single-instance window reuse.

use std::{
    io::{BufRead as _, BufReader, Write as _},
    path::PathBuf,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use futures::StreamExt as _;
use gpui_kit::{AnyWindowHandle, App, WeakEntity};

use crate::{cli::LaunchConfig, workspace::Workspace};

/// Returns the platform-specific socket path used for IPC.
pub(crate) fn socket_path() -> PathBuf {
    #[cfg(unix)]
    {
        if let Some(flatpak_id) = std::env::var_os("FLATPAK_ID")
            && let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR")
        {
            let app_dir = PathBuf::from(runtime_dir).join("app").join(flatpak_id);
            let _ = std::fs::create_dir_all(&app_dir);
            return app_dir.join("downmark.sock");
        }
        if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR") {
            return PathBuf::from(runtime_dir).join("downmark.sock");
        }
        let user = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        std::env::temp_dir().join(format!("downmark-{user}.sock"))
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("downmark.sock")
    }
}

/// Attempts to forward the launch configuration to an already running instance.
///
/// Returns `true` if an existing instance received the message and accepted it.
/// Returns `false` if no instance is running or the connection failed.
#[must_use]
pub(crate) fn try_forward_to_existing(config: &LaunchConfig) -> bool {
    #[cfg(unix)]
    {
        let path = socket_path();
        if !path.exists() {
            return false;
        }

        let Ok(mut stream) = UnixStream::connect(&path) else {
            // Stale socket from previous crash; remove it so we can bind
            let _ = std::fs::remove_file(&path);
            return false;
        };

        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_read_timeout(Some(Duration::from_millis(1000)));

        let Ok(payload) = serde_json::to_string(config) else {
            return false;
        };
        if stream.write_all(payload.as_bytes()).is_err()
            || stream.write_all(b"\n").is_err()
            || stream.flush().is_err()
        {
            return false;
        }

        let mut ack = [0u8; 2];
        let Ok(n) = std::io::Read::read(&mut stream, &mut ack) else {
            return false;
        };
        n == 2 && &ack == b"ok"
    }
    #[cfg(not(unix))]
    {
        let _ = config;
        false
    }
}

/// Spawns the IPC listener that accepts incoming commands and dispatches them
/// to the workspace window.
pub(crate) fn start_server(
    window_handle: AnyWindowHandle,
    workspace: WeakEntity<Workspace>,
    cx: &mut App,
) {
    #[cfg(unix)]
    {
        let path = socket_path();
        // Remove any abandoned socket from previous termination
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }

        let Ok(listener) = UnixListener::bind(&path) else {
            return;
        };

        let (ipc_tx, mut ipc_rx) = futures::channel::mpsc::unbounded::<LaunchConfig>();

        std::thread::Builder::new()
            .name("downmark-ipc".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else {
                        break;
                    };
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(1000)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));

                    let mut reader = BufReader::new(&mut stream);
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_ok()
                        && let Ok(config) = serde_json::from_str::<LaunchConfig>(&line)
                    {
                        let _ = stream.write_all(b"ok");
                        let _ = stream.flush();
                        if ipc_tx.unbounded_send(config).is_err() {
                            break;
                        }
                    }
                }
                let _ = std::fs::remove_file(&path);
            })
            .expect("IPC listener thread must spawn");

        cx.spawn(async move |cx| {
            while let Some(config) = ipc_rx.next().await {
                let ws = workspace.clone();
                cx.update(|cx| {
                    _ = window_handle.update(cx, |_, window, cx| {
                        if let Some(workspace) = ws.upgrade() {
                            workspace.update(cx, |workspace, cx| {
                                workspace.open_launch_config(config, window, cx);
                            });
                        }
                        window.activate_window();
                    });
                });
            }
        })
        .detach();
    }
    #[cfg(not(unix))]
    {
        let _ = window_handle;
        let _ = workspace;
    }
}

/// Removes the IPC socket file upon normal application exit.
pub(crate) fn cleanup_socket() {
    #[cfg(unix)]
    {
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_socket_path_not_empty() {
        let path = socket_path();
        assert!(path.to_string_lossy().contains("downmark"));
    }

    #[test]
    fn test_launch_config_serialization() {
        let config = LaunchConfig {
            files: vec![crate::cli::FileTarget {
                path: PathBuf::from("test.md"),
                target: Some(crate::cli::Target::Line(42)),
            }],
            layout: Some(crate::settings::Layout::Split),
            glance: crate::workspace::GlanceStart::Normal,
            new_window: false,
        };

        let json = serde_json::to_string(&config).expect("must serialize");
        let deserialized: LaunchConfig = serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_ipc_socket_roundtrip() {
        let test_socket =
            std::env::temp_dir().join(format!("downmark-test-{}.sock", std::process::id()));
        if test_socket.exists() {
            let _ = std::fs::remove_file(&test_socket);
        }

        let listener = UnixListener::bind(&test_socket).expect("must bind test socket");
        let test_socket_clone = test_socket;

        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(&mut stream);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read line");
            let config = serde_json::from_str::<LaunchConfig>(&line).expect("deserialize");
            stream.write_all(b"ok").expect("write ack");
            stream.flush().expect("flush ack");
            config
        });

        let mut client_stream = UnixStream::connect(&test_socket_clone).expect("connect");
        let config = LaunchConfig {
            files: vec![crate::cli::FileTarget {
                path: PathBuf::from("test.md"),
                target: Some(crate::cli::Target::Line(42)),
            }],
            layout: Some(crate::settings::Layout::Split),
            glance: crate::workspace::GlanceStart::Normal,
            new_window: false,
        };
        serde_json::to_writer(&mut client_stream, &config).expect("send config");
        client_stream.write_all(b"\n").expect("send newline");
        client_stream.flush().expect("flush client");

        let mut ack = [0u8; 2];
        std::io::Read::read_exact(&mut client_stream, &mut ack).expect("read ack");
        assert_eq!(&ack, b"ok");

        let received = handle.join().expect("join");
        assert_eq!(config, received);
        let _ = std::fs::remove_file(&test_socket_clone);
    }
}
