//! For whoever writes a plugin: `nadi-plugin test <folder>` runs its `test.luau` against its script; `nadi-plugin run <folder> <event.json>`
//! calls it once and prints what it answers.

use nadi_core::luau::{run_tests, Limits, Script};
use nadi_core::manifest;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match (args.get(1).map(String::as_str), args.get(2)) {
        (Some("test"), Some(dir)) => test(Path::new(dir)),
        (Some("run"), Some(dir)) => run(Path::new(dir), args.get(3).map(String::as_str)),
        _ => {
            eprintln!("nadi-plugin test <folder>\nnadi-plugin run <folder> [event.json]");
            2
        }
    };
    std::process::exit(code);
}

fn load(dir: &Path) -> Result<(manifest::Manifest, String), String> {
    let (m, problem) = manifest::read(dir).ok_or("There is no manifest.json there.")?;
    if let Some(p) = problem {
        return Err(p);
    }
    let src = std::fs::read_to_string(dir.join(&m.main)).map_err(|e| e.to_string())?;
    Ok((m, src))
}

fn test(dir: &Path) -> i32 {
    let (m, src) = match load(dir) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let tests = match std::fs::read_to_string(dir.join("test.luau")) {
        Ok(t) => t,
        Err(_) => {
            eprintln!("{} has no test.luau.", m.id);
            return 1;
        }
    };
    match run_tests(&m.id, &src, &tests, Limits { call: std::time::Duration::from_secs(2), memory: 64 << 20 }) {
        Ok(results) => {
            let mut failed = 0;
            for (name, fails) in &results {
                if fails.is_empty() {
                    println!("ok    {name}");
                } else {
                    failed += 1;
                    println!("FAIL  {name}");
                    for f in fails {
                        println!("        {f}");
                    }
                }
            }
            println!("{}: {} passed, {} failed", m.id, results.len() - failed, failed);
            i32::from(failed > 0)
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

fn run(dir: &Path, event: Option<&str>) -> i32 {
    let (m, src) = match load(dir) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    };
    let ev: serde_json::Value = match event.map(std::fs::read_to_string) {
        Some(Ok(t)) => serde_json::from_str(&t).unwrap_or_default(),
        _ => serde_json::json!({ "event": "start", "now": 0, "settings": {} }),
    };
    match Script::load(&m.id, &src, Limits::default()).and_then(|s| s.call(&ev)) {
        Ok(r) => {
            println!("{}", serde_json::to_string_pretty(&r).unwrap_or_default());
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}
