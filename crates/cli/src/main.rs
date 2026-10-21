use std::io::{BufRead, Write};
use std::process::ExitCode;

const USAGE: &str = "usage:
  wirebench det                  print golden trace hashes (CI diffs this per target)
  wirebench run <script>         run a scenario script and print its transcript
  wirebench shell <device>       interactive shell on pc1|pc2|r1|r2 of the blank chain";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["det"] => {
            let (w, _, _) = engine::scenarios::two_hosts_ping();
            println!("two_hosts_ping {:016x}", w.trace_hash());
            let c = engine::chain::static_chain_ping();
            println!("static_chain_ping {:016x}", c.world.trace_hash());
            let script = include_str!("../../../testkit/scenarios/m1_static_routes.script");
            let out = engine::script::run_script(script).expect("builtin script");
            let hash = out
                .lines()
                .last()
                .and_then(|l| l.strip_prefix("--- trace "))
                .unwrap_or("?");
            println!("m1_static_routes {hash}");
            let lab = include_str!("../../../labs/m1_static_routes.lab.toml");
            let mut b =
                engine::lab::load_and_build(lab, kernel::Trace::keeping()).expect("builtin lab");
            let pc1 = b.devices[2].id;
            b.world.with_device::<netdev::Host, _>(pc1, |h, c| {
                h.ping(c, wire::Ipv4Addr::new(10, 0, 2, 2))
            });
            b.world.run_to_idle();
            println!("lab_static_routes {:016x}", b.world.trace_hash());
            ExitCode::SUCCESS
        }
        ["run", path] => match std::fs::read_to_string(path) {
            Ok(text) => match engine::script::run_script(&text) {
                Ok(out) => {
                    print!("{out}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("script error: {e}");
                    ExitCode::FAILURE
                }
            },
            Err(e) => {
                eprintln!("{path}: {e}");
                ExitCode::FAILURE
            }
        },
        ["bench"] => bench(),
        ["shell", dev] => shell(dev),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// Kernel throughput: pc1 pings pc2 across two routers, many times, hash-only trace.
fn bench() -> ExitCode {
    use netdev::Host;
    use std::time::Instant;
    const PINGS: u32 = 20_000;
    let mut c = engine::chain::static_chain_with(kernel::Trace::new());
    let dst = wire::Ipv4Addr::new(10, 0, 2, 2);
    let start = Instant::now();
    for _ in 0..PINGS {
        c.world
            .with_device::<Host, _>(c.pc1, |h, ctx| h.ping(ctx, dst));
        c.world.run_to_idle();
    }
    let secs = start.elapsed().as_secs_f64();
    let events = c.world.kernel.delivered();
    let ok = c.world.device::<Host>(c.pc1).ping_results.len();
    println!("pings        {PINGS} ({ok} answered)");
    println!("events       {events}");
    println!("wall         {secs:.3} s");
    println!("events/s     {:.0}", events as f64 / secs);
    println!("pings/s      {:.0}", PINGS as f64 / secs);
    println!("virtual time {:.1} s", c.world.kernel.now().0 as f64 / 1e9);
    ExitCode::SUCCESS
}

/// Line-mode shell: a trailing `?` shows help; Tab completion needs a raw terminal and
/// lives in the GUI terminal widget.
fn shell(dev: &str) -> ExitCode {
    use engine::shell::Shell;
    let mut chain = engine::chain::blank_chain();
    let mut sh = match dev {
        "r1" => Shell::router(chain.r1),
        "r2" => Shell::router(chain.r2),
        "pc1" => Shell::host(chain.pc1),
        "pc2" => Shell::host(chain.pc2),
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let stdin = std::io::stdin();
    loop {
        print!("{}", sh.prompt(&chain.world));
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            return ExitCode::SUCCESS;
        }
        let line = line.trim_end();
        let text = match line.strip_suffix('?') {
            Some(l) => Some(sh.help(l)),
            None => sh.exec(&mut chain.world, line),
        };
        match text {
            Some(t) if !t.is_empty() => println!("{t}"),
            Some(_) => {}
            None => return ExitCode::SUCCESS,
        }
    }
}
