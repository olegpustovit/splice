//! Policy gate (PLAN.md T0.3 fills it in): shape rules, crate deps allowlist, docs, registry parity.
fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    eprintln!("gate {what}: not implemented yet (PLAN.md T0.3)");
}
