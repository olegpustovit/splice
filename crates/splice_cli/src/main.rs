//! The splice command: headless core, MCP stdio server, apply/snapshot/describe/export/friction, and `splice open` for the minimal viewer.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("mcp") if args.len() == 1 => {
            use std::io::{self, ErrorKind};
            if let Err(error) = splice_mcp::serve(&mut io::stdin().lock(), &mut io::stdout().lock())
                && error.kind() != ErrorKind::BrokenPipe
            {
                eprintln!("splice mcp: {error}");
                std::process::exit(1);
            }
        }
        Some("--version") | Some("version") => println!("splice {}", env!("CARGO_PKG_VERSION")),
        _ => {
            eprintln!(
                "usage: splice <mcp|apply|snapshot|describe|export|open|friction|version> ... (see PLAN.md T1.x)"
            );
            std::process::exit(2);
        }
    }
}
