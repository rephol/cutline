use cutline::config::{AppendScope, Config};
use std::path::Path;
use std::process::ExitCode;

const USAGE: &str = "usage: cutline replay <dir> [options]

  --budget N            cliff fires above this many estimated tokens (default 100000)
  --target N            cliff cuts down to this many (default: half of budget)
  --keep N              last N turns are never touched (default 6)
  --min-stub-bytes N    results at or below this stay verbatim (default 500)
  --max-result-bytes N  append mode caps results larger than this (default 40000)
  --scope newest|all    which tool_results append mode may rewrite (default newest)
  --no-cliff            disable approach 1
  --no-append           disable approach 2";

fn parse(args: &[String]) -> Result<Config, String> {
    let mut c = Config::default();
    let mut target_set = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let val = |i: &mut usize| -> Result<String, String> {
            *i += 1;
            args.get(*i).cloned().ok_or(format!("{a} needs a value"))
        };
        let num = |s: String| s.parse::<u64>().map_err(|_| format!("{a}: bad number {s}"));
        match a {
            "--budget" => c.budget = num(val(&mut i)?)?,
            "--target" => {
                c.target = num(val(&mut i)?)?;
                target_set = true;
            }
            "--keep" => c.keep_recent_turns = num(val(&mut i)?)? as usize,
            "--min-stub-bytes" => c.min_stub_bytes = num(val(&mut i)?)? as usize,
            "--max-result-bytes" => c.append.max_result_bytes = num(val(&mut i)?)? as usize,
            "--scope" => {
                c.append.scope = match val(&mut i)?.as_str() {
                    "newest" => AppendScope::Newest,
                    "all" => AppendScope::All,
                    o => return Err(format!("--scope: expected newest|all, got {o}")),
                }
            }
            "--no-cliff" => c.cliff = false,
            "--no-append" => c.append.enabled = false,
            o => return Err(format!("unknown option {o}")),
        }
        i += 1;
    }
    if !target_set {
        c.target = c.budget / 2;
    }
    Ok(c)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("replay") if args.len() >= 2 => {
            let cfg = match parse(&args[2..]) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("cutline: {e}\n{USAGE}");
                    return ExitCode::from(2);
                }
            };
            match cutline::replay::replay_dir(Path::new(&args[1]), &cfg) {
                Ok(s) => {
                    print!("{}", s.render());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("cutline: cannot read {}: {e}", args[1]);
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
