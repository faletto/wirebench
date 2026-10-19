use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Prints the canonical trace hash of the golden scenario. CI diffs this output
        // against testkit/determinism.txt on every target.
        Some("det") => {
            let (w, _, _) = engine::scenarios::two_hosts_ping();
            println!("two_hosts_ping {:016x}", w.trace_hash());
            let c = engine::chain::static_chain_ping();
            println!("static_chain_ping {:016x}", c.world.trace_hash());
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("usage: ptclone det");
            ExitCode::from(2)
        }
    }
}
