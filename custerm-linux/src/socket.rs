use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc;

use gtk4::ApplicationWindow;
use serde_json::json;

use custerm_core::protocol::{Request, Response};

use crate::tabs::TabManager;

pub struct SocketCommand {
    pub request: Request,
    pub reply: std::sync::mpsc::Sender<Response>,
}

pub fn start_server(socket_path: &str) -> mpsc::Receiver<SocketCommand> {
    let (tx, rx) = mpsc::channel();

    // Remove stale socket
    let _ = std::fs::remove_file(socket_path);

    let listener = match UnixListener::bind(socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[custerm] failed to bind socket at {socket_path}: {e}");
            return rx;
        }
    };

    eprintln!("[custerm] socket server listening at {socket_path}");

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[custerm] socket accept error: {e}");
                    continue;
                }
            };

            let tx = tx.clone();
            std::thread::spawn(move || {
                let reader = match stream.try_clone() {
                    Ok(s) => BufReader::new(s),
                    Err(e) => {
                        eprintln!("[custerm] socket clone error: {e}");
                        return;
                    }
                };
                let mut writer = stream;

                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    if line.is_empty() {
                        continue;
                    }

                    let request: Request = match serde_json::from_str(&line) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = Response::error(
                                String::new(),
                                "parse_error",
                                &format!("Invalid JSON: {e}"),
                            );
                            let _ = writeln!(writer, "{}", serde_json::to_string(&err).unwrap());
                            let _ = writer.flush();
                            continue;
                        }
                    };

                    let (reply_tx, reply_rx) = mpsc::channel();
                    let cmd = SocketCommand {
                        request,
                        reply: reply_tx,
                    };

                    if tx.send(cmd).is_err() {
                        break;
                    }

                    match reply_rx.recv() {
                        Ok(response) => {
                            let _ =
                                writeln!(writer, "{}", serde_json::to_string(&response).unwrap());
                            let _ = writer.flush();
                        }
                        Err(_) => break,
                    }
                }
            });
        }
    });

    rx
}

pub fn dispatch(
    req: &Request,
    mgr: &Rc<TabManager>,
    window: &ApplicationWindow,
) -> Response {
    match req.method.as_str() {
        "system.ping" => Response::success(req.id.clone(), json!({ "status": "ok" })),

        "background.set" => {
            let path = req.params.get("path").and_then(|v| v.as_str());
            match path {
                Some(p) => {
                    let path = Path::new(p);
                    if !path.exists() {
                        return Response::error(
                            req.id.clone(),
                            "not_found",
                            &format!("File not found: {p}"),
                        );
                    }
                    if let Some(panel) = mgr.active_panel() {
                        panel.set_background(path);
                        Response::success(req.id.clone(), json!({ "status": "ok" }))
                    } else {
                        Response::error(req.id.clone(), "no_panel", "No active panel")
                    }
                }
                None => Response::error(req.id.clone(), "invalid_params", "Missing 'path' param"),
            }
        }

        "background.clear" => {
            if let Some(panel) = mgr.active_panel() {
                panel.clear_background();
                Response::success(req.id.clone(), json!({ "status": "ok" }))
            } else {
                Response::error(req.id.clone(), "no_panel", "No active panel")
            }
        }

        "background.set_tint" => {
            let opacity = req.params.get("opacity").and_then(|v| v.as_f64());
            match opacity {
                Some(o) => {
                    if let Some(panel) = mgr.active_panel() {
                        panel.set_tint(o);
                        Response::success(req.id.clone(), json!({ "status": "ok" }))
                    } else {
                        Response::error(req.id.clone(), "no_panel", "No active panel")
                    }
                }
                None => {
                    Response::error(req.id.clone(), "invalid_params", "Missing 'opacity' param")
                }
            }
        }

        "tab.new" => {
            mgr.add_tab(window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "tab.close" => {
            mgr.close_focused(window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "tab.list" => {
            let count = mgr.tab_count();
            let current = mgr.current_tab();
            Response::success(
                req.id.clone(),
                json!({ "count": count, "current": current }),
            )
        }

        "split.horizontal" => {
            mgr.split_focused(gtk4::Orientation::Horizontal, window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        "split.vertical" => {
            mgr.split_focused(gtk4::Orientation::Vertical, window);
            Response::success(req.id.clone(), json!({ "status": "ok" }))
        }

        _ => Response::error(
            req.id.clone(),
            "unknown_method",
            &format!("Unknown method: {}", req.method),
        ),
    }
}

pub fn cleanup(socket_path: &str) {
    let _ = std::fs::remove_file(socket_path);
}
