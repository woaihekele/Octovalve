use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn main() {
    eprintln!("=== Test Start ===");
    let mut child = Command::new("/usr/local/bin/codex-acp")
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
        .spawn().expect("Failed");
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel::<String>();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().flatten() { 
            eprintln!("[RECV] {}", if line.len() > 100 { &line[..100] } else { &line });
            let _ = tx.send(line); 
        }
    });

    let send = |stdin: &mut std::process::ChildStdin, req: &str, id: u64, rx: &mpsc::Receiver<String>| -> serde_json::Value {
        eprintln!("[SEND] id={}", id);
        writeln!(stdin, "{}", req).unwrap(); stdin.flush().unwrap();
        loop {
            if let Ok(line) = rx.recv_timeout(Duration::from_secs(30)) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                    if json.get("id").is_none() { continue; }
                    if json["id"].as_u64() == Some(id) { return json; }
                }
            } else { panic!("Timeout"); }
        }
    };

    send(&mut stdin, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1","clientCapabilities":{"prompt":{"embeddedContext":true,"image":true}},"clientInfo":{"name":"t","version":"0.1"}}}"#, 1, &rx);
    send(&mut stdin, r#"{"jsonrpc":"2.0","id":2,"method":"authenticate","params":{"methodId":"openai-api-key"}}"#, 2, &rx);
    let r = send(&mut stdin, r#"{"jsonrpc":"2.0","id":3,"method":"session/new","params":{"cwd":"/tmp","mcpServers":[]}}"#, 3, &rx);
    let sid = r["result"]["sessionId"].as_str().unwrap();
    eprintln!("[SID] {}", sid);

    let req = format!(r#"{{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{{"sessionId":"{}","prompt":[{{"type":"text","text":"say hi"}}]}}}}"#, sid);
    eprintln!("[SEND] prompt");
    writeln!(stdin, "{}", req).unwrap(); stdin.flush().unwrap();

    eprintln!("=== Waiting ===");
    let t = std::time::Instant::now();
    while t.elapsed() < Duration::from_secs(30) {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(line) => {
                if let Ok(j) = serde_json::from_str::<serde_json::Value>(&line) {
                    if j["id"].as_u64() == Some(4) { eprintln!("=== DONE ==="); break; }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(_) => break,
        }
    }
    drop(stdin); let _ = child.kill();
}
