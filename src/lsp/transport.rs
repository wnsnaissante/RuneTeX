use super::*;
use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command as Process, Stdio},
    time::{Duration, Instant},
};

pub(super) fn write_message(output: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

pub(super) fn read_message(input: &mut impl BufRead) -> io::Result<Value> {
    let mut length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        header_bytes += line.len();
        if header_bytes > 16_384 {
            return Err(io::ErrorKind::InvalidData.into());
        }
        if line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("Content-Length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let length = length
        .filter(|length| *length <= 32 * 1024 * 1024)
        .ok_or(io::ErrorKind::InvalidData)?;
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(Into::into)
}

struct Server {
    child: Child,
    input: ChildStdin,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(super) fn run(
    root: PathBuf,
    documents: Vec<(String, String)>,
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<Event>,
    wake: Waker,
) {
    let report = |event| {
        let _ = events.send(event);
        wake();
    };
    if let Err(error) = session(&root, documents, commands, &report) {
        report(Event::Status(format!("texlab: {error}")));
    }
}

fn session(
    root: &Path,
    documents: Vec<(String, String)>,
    commands: mpsc::Receiver<Command>,
    report: &impl Fn(Event),
) -> Result<(), String> {
    let path = server_path()
        .ok_or("texlab을 찾을 수 없습니다 · 설치 경로 또는 RUNETEX_TEXLAB 설정 확인")?;
    std::fs::create_dir_all(root.join(".runetex")).map_err(|e| e.to_string())?;
    let stderr =
        std::fs::File::create(root.join(".runetex/texlab.log")).map_err(|e| e.to_string())?;
    let mut process = Process::new(path);
    process
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }
    let mut child = process.spawn().map_err(|e| e.to_string())?;
    let input = child.stdin.take().ok_or("stdin 열기 실패")?;
    let output = child.stdout.take().ok_or("stdout 열기 실패")?;
    let mut server = Server { child, input };
    let (incoming, receive) = mpsc::channel();
    std::thread::spawn(move || {
        let mut input = BufReader::new(output);
        loop {
            let value = read_message(&mut input);
            let ended = value.is_err();
            if incoming.send(value).is_err() || ended {
                break;
            }
        }
    });
    let root_uri = uri(root, "")?;
    let configuration = json!({"build":{"onSave":false},"chktex":{"onOpenAndSave":false,"onEdit":false},"diagnosticsDelay":200});
    let mut documents: HashMap<String, (String, i32)> =
        documents.into_iter().map(|(p, t)| (p, (t, 1))).collect();
    let mut pending: HashMap<u64, (Query, Context)> = HashMap::new();
    let mut queue = std::collections::VecDeque::new();
    let mut next_id = 2;
    let mut initialized = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    write_message(&mut server.input, &json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "processId":std::process::id(),"clientInfo":{"name":"RuneTeX","version":env!("CARGO_PKG_VERSION")},
        "rootUri":root_uri,"workspaceFolders":[{"uri":root_uri,"name":"RuneTeX"}],
        "capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"configuration":true},
            "textDocument":{"completion":{"completionItem":{"snippetSupport":false}},"hover":{"contentFormat":["plaintext"]},"publishDiagnostics":{"versionSupport":true}}}
    }})).map_err(|e|e.to_string())?;
    loop {
        while let Ok(message) = receive.try_recv() {
            let value = message.map_err(|e| format!("연결 종료 ({e})"))?;
            if let Some(method) = value["method"].as_str() {
                if let Some(id) = value.get("id") {
                    let result = if method == "workspace/configuration" {
                        Value::Array(
                            value["params"]["items"]
                                .as_array()
                                .map(|items| {
                                    items
                                        .iter()
                                        .map(|item| {
                                            let section =
                                                item["section"].as_str().unwrap_or("texlab");
                                            section
                                                .split('.')
                                                .skip(1)
                                                .fold(configuration.clone(), |node, key| {
                                                    node[key].clone()
                                                })
                                        })
                                        .collect()
                                })
                                .unwrap_or_default(),
                        )
                    } else {
                        Value::Null
                    };
                    write_message(
                        &mut server.input,
                        &json!({"jsonrpc":"2.0","id":id,"result":result}),
                    )
                    .map_err(|e| e.to_string())?;
                } else if method == "textDocument/publishDiagnostics" {
                    let params = &value["params"];
                    if let Some(path) = params["uri"]
                        .as_str()
                        .and_then(|uri| relative_path(root, uri))
                    {
                        if let Some(version) = params["version"].as_i64()
                            && documents
                                .get(&path)
                                .is_some_and(|(_, current)| version < i64::from(*current))
                        {
                            continue;
                        }
                        let diagnostics = params["diagnostics"]
                            .as_array()
                            .map(|items| {
                                items
                                    .iter()
                                    .map(|item| Diagnostic {
                                        range: item["range"].clone(),
                                        message: item["message"].as_str().unwrap_or("").into(),
                                        warning: item["severity"].as_i64() != Some(1),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        report(Event::Diagnostics(path, diagnostics));
                    }
                }
            } else if let Some(id) = value["id"].as_u64() {
                if id == 1 {
                    if let Some(error) = value.get("error") {
                        return Err(error.to_string());
                    }
                    initialized = true;
                    notify(&mut server.input, "initialized", json!({}))?;
                    notify(
                        &mut server.input,
                        "workspace/didChangeConfiguration",
                        json!({"settings":{"texlab":configuration}}),
                    )?;
                    for (path, (text, version)) in &documents {
                        open(&mut server.input, root, path, text, *version)?;
                    }
                    let version = value["result"]["serverInfo"]["version"]
                        .as_str()
                        .unwrap_or("");
                    report(Event::Status(format!("texlab {version} 연결됨")));
                } else if let Some((kind, context)) = pending.remove(&id) {
                    if let Some(error) = value.get("error") {
                        report(Event::Status(format!(
                            "texlab 요청 실패: {}",
                            error["message"].as_str().unwrap_or("오류")
                        )));
                    } else {
                        report(Event::Response(kind, context, value["result"].clone()));
                    }
                }
            }
        }
        if !initialized && Instant::now() > deadline {
            return Err("초기화 시간 초과".into());
        }
        let command = if initialized && !queue.is_empty() {
            queue.pop_front()
        } else {
            match commands.recv_timeout(Duration::from_millis(20)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => Some(Command::Shutdown),
            }
        };
        let Some(command) = command else {
            continue;
        };
        if matches!(command, Command::Shutdown) {
            let _ = write_message(
                &mut server.input,
                &json!({"jsonrpc":"2.0","id":next_id,"method":"shutdown","params":null}),
            );
            let shutdown_deadline = Instant::now() + Duration::from_millis(150);
            while Instant::now() < shutdown_deadline {
                if let Ok(Ok(value)) = receive.recv_timeout(Duration::from_millis(20))
                    && value["id"] == next_id
                {
                    break;
                }
            }
            let _ = notify(&mut server.input, "exit", Value::Null);
            return Ok(());
        }
        if !initialized {
            if queue.len() < 128 {
                queue.push_back(command);
            }
            continue;
        }
        match command {
            Command::Sync(path, text) => {
                sync(&mut server.input, root, &mut documents, &path, &text)?
            }
            Command::Saved(path) => notify(
                &mut server.input,
                "textDocument/didSave",
                json!({"textDocument":{"uri":uri(root,&path)?}}),
            )?,
            Command::Query(kind, context) => {
                sync(
                    &mut server.input,
                    root,
                    &mut documents,
                    &context.path,
                    &context.text,
                )?;
                let method = match kind {
                    Query::Completion => "textDocument/completion",
                    Query::Hover => "textDocument/hover",
                    Query::Definition => "textDocument/definition",
                };
                write_message(&mut server.input,&json!({"jsonrpc":"2.0","id":next_id,"method":method,"params":{
                    "textDocument":{"uri":uri(root,&context.path)?},"position":position(&context.text,context.cursor)
                }})).map_err(|e|e.to_string())?;
                if pending.len() >= 16
                    && let Some(oldest) = pending.keys().min().copied()
                {
                    pending.remove(&oldest);
                }
                pending.insert(next_id, (kind, context));
                next_id += 1;
            }
            Command::Shutdown => unreachable!(),
        }
    }
}

fn notify(input: &mut ChildStdin, method: &str, params: Value) -> Result<(), String> {
    write_message(
        input,
        &json!({"jsonrpc":"2.0","method":method,"params":params}),
    )
    .map_err(|e| e.to_string())
}
fn open(
    input: &mut ChildStdin,
    root: &Path,
    path: &str,
    text: &str,
    version: i32,
) -> Result<(), String> {
    notify(
        input,
        "textDocument/didOpen",
        json!({"textDocument":{"uri":uri(root,path)?,"languageId":if path.ends_with(".bib") {"bibtex"} else {"latex"},"version":version,"text":text}}),
    )
}
fn sync(
    input: &mut ChildStdin,
    root: &Path,
    documents: &mut HashMap<String, (String, i32)>,
    path: &str,
    text: &str,
) -> Result<(), String> {
    if let Some((old, version)) = documents.get_mut(path) {
        if old != text {
            *old = text.into();
            *version += 1;
            notify(
                input,
                "textDocument/didChange",
                json!({"textDocument":{"uri":uri(root,path)?,"version":version},"contentChanges":[{"text":text}]}),
            )?;
        }
    } else {
        open(input, root, path, text, 1)?;
        documents.insert(path.into(), (text.into(), 1));
    }
    Ok(())
}
