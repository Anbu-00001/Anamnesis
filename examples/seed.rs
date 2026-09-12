//! Write the demo ledger to a file.
//!
//!     cargo run --example seed -- seed.json
//!
//! The ledger itself lives in `anamnesis::demo`, so that `ana demo` and this
//! generator can never drift apart.

use std::path::Path;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "seed.json".into());
    let ledger = anamnesis::demo::ledger();
    anamnesis::store::save(Path::new(&path), &ledger).expect("write seed");
    let resolved = ledger.claims.iter().filter(|c| c.is_resolved()).count();
    let open = ledger.claims.len() - resolved;
    eprintln!(
        "wrote {} claims ({resolved} resolved, {open} open) to {path}",
        ledger.claims.len()
    );
}
