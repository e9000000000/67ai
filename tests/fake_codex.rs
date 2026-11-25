// Deterministic executable used by integration.mjs to test the queue without model calls.
use std::{io::{BufRead, Read, Write}, time::Duration};
fn main() {
    if std::env::args().any(|a| a == "app-server") {
        for line in std::io::stdin().lock().lines() {
            let line = line.unwrap();
            if line.contains("\"initialize\"") {
                println!(r#"{{"id":1,"result":{{}}}}"#);
            } else if line.contains("account/rateLimits/read") {
                let dir = std::path::PathBuf::from(std::env::var("67AI_DATA_DIR").unwrap());
                if let Ok(reset) = std::fs::read_to_string(dir.join("limit-reset")) {
                    println!("{{\"id\":2,\"result\":{{\"rateLimits\":{{\"primary\":{{\"usedPercent\":100,\"resetsAt\":{reset}}}}}}}}}");
                    std::io::stdout().flush().unwrap();
                    continue;
                }
                println!(r#"{{"id":2,"result":{{"rateLimits":{{"primary":{{"usedPercent":15,"windowDurationMins":300,"resetsAt":1900000000}},"secondary":{{"usedPercent":37,"windowDurationMins":10080,"resetsAt":1900500000}}}}}}}}"#);
            } else if line.contains("thread/delete") {
                if line.contains("fail-delete") {
                    println!(r#"{{"id":2,"error":{{"message":"Deletion failed"}}}}"#);
                } else {
                    let dir = std::env::var("67AI_DATA_DIR").unwrap();
                    std::fs::write(std::path::Path::new(&dir).join("deleted-session.json"), line).unwrap();
                    println!(r#"{{"id":2,"result":{{}}}}"#);
                }
            }
            std::io::stdout().flush().unwrap();
        }
        return;
    }
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).unwrap();
    let args: Vec<String> = std::env::args().collect();
    assert!(args.iter().any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    assert!(!args.iter().any(|a| a == "--sandbox" || a.starts_with("approval_policy=")));
    if prompt.contains("You are a separate AI reviewer.") {
        let args: Vec<String> = std::env::args().collect();
        assert!(!args.iter().any(|a| a == "resume"));
        assert!(args.windows(2).any(|a| a == ["--model", "hard-review-model"]));
        assert!(prompt.contains("actually exercise the changed program behavior"));
        assert!(prompt.contains("inspect its rendered appearance"));
        assert!(prompt.contains("Do not edit implementation files"));
        assert!(prompt.contains("Report what you tested"));
        assert!(prompt.contains("67ai executes your verdict automatically"));
        let schema_index = args.iter().position(|a| a == "--output-schema").unwrap();
        let schema = std::fs::read_to_string(&args[schema_index + 1]).unwrap();
        assert!(schema.contains("\"approve\"") && schema.contains("\"reject\""));
        assert!(!schema.contains("\"completed\""));
        assert!(!prompt.contains("relevant read-only checks"));
        println!(r#"{{"type":"thread.started","thread_id":"review-session"}}"#);
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(900));
        if prompt.contains("REVIEW_FAIL") { std::process::exit(1); }
        if prompt.contains("REVIEW_INVALID") {
            println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"No decision"}}}}"#);
        } else {
            let decision = if prompt.contains("REVIEW_FEEDBACK") { "reject" } else { "approve" };
            println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"decision\":\"{decision}\",\"message\":\"Review verdict\"}}"}}}}"#);
            println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"Review finished."}}}}"#);
        }
        println!(r#"{{"type":"turn.completed"}}"#);
        return;
    }
    if prompt.starts_with("Task: Resolve parallel task merge conflicts\n") {
        let args: Vec<String> = std::env::args().collect();
        assert!(!args.iter().any(|a| a == "resume"));
        assert!(!prompt.contains("Do not switch branches, merge into main"));
        assert!(prompt.contains("Original task instructions:"));
        println!(r#"{{"type":"thread.started","thread_id":"merge-repair"}}"#);
        if args.iter().any(|a| a == "merge-fail") {
            println!(r#"{{"type":"turn.failed","error":{{"message":"Resolution failed"}}}}"#);
            std::process::exit(1);
        }
        if !args.iter().any(|a| a == "merge-noop") {
            std::fs::write("base.txt", "main edit\ntask edit\n").unwrap();
            assert!(std::process::Command::new("git").args(["add", "base.txt"]).status().unwrap().success());
        }
        println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"outcome\":\"completed\",\"message\":\"Resolved\"}}"}}}}"#);
        println!(r#"{{"type":"turn.completed"}}"#);
        return;
    }
    if prompt.contains("PARALLEL_TEST") {
        let title = prompt.lines().next().unwrap().strip_prefix("Task: ").unwrap();
        std::fs::write(format!("{title}.txt"), &prompt).unwrap();
        println!(r#"{{"type":"thread.started","thread_id":"parallel-test"}}"#);
        std::io::stdout().flush().unwrap();
        std::thread::sleep(Duration::from_millis(1600));
        println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"outcome\":\"completed\",\"message\":\"Done\"}}"}}}}"#);
        println!(r#"{{"type":"turn.completed"}}"#);
        return;
    }
    std::fs::write("last-prompt.txt", &prompt).unwrap();
    let args: Vec<String> = std::env::args().collect();
    assert!(args.iter().any(|a| a == "--dangerously-bypass-approvals-and-sandbox"));
    let schema_index = args.iter().position(|a| a == "--output-schema").expect("Missing response schema");
    assert!(std::fs::read_to_string(&args[schema_index + 1]).unwrap().contains("refused"));
    std::fs::write("last-args.json", format!("{args:?}")).unwrap();
    println!(r#"{{"type":"thread.started","thread_id":"11111111-1111-4111-8111-111111111111"}}"#);
    std::io::stdout().flush().unwrap();
    if prompt.contains("LIMIT_TEST") && !std::path::Path::new("limit-hit").exists() {
        std::fs::write("limit-hit", "").unwrap();
        let reset = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 5;
        let dir = std::path::PathBuf::from(std::env::var("67AI_DATA_DIR").unwrap());
        std::fs::write(dir.join("limit-reset"), reset.to_string()).unwrap();
        println!(r#"{{"type":"turn.failed","error":{{"message":"You've hit your usage limit"}}}}"#);
        std::process::exit(1);
    }
    if prompt.contains("WAIT_TEST") { std::thread::sleep(Duration::from_secs(60)); }
    std::thread::sleep(Duration::from_millis(700));
    if prompt.contains("FAIL_TEST") {
        println!(r#"{{"type":"turn.failed","error":{{"message":"Deliberate test failure"}}}}"#);
        std::process::exit(1);
    }
    let outcome = if prompt.contains("ANSWER_TEST") { "completed" }
        else if prompt.contains("QUESTION_TEST") { "question" }
        else if prompt.contains("BLOCKED_TEST") { "blocked" }
        else if prompt.contains("REFUSED_TEST") { "refused" }
        else { "completed" };
    if prompt.contains("UNSTRUCTURED_TEST") {
        println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"Unexpected plain reply"}}}}"#);
    } else {
        println!(r#"{{"type":"item.completed","item":{{"type":"agent_message","text":"{{\"outcome\":\"{outcome}\",\"message\":\"Test response\"}}"}}}}"#);
    }
    println!(r#"{{"type":"turn.completed","usage":{{}}}}"#);
}
