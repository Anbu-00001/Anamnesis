//! Old and new metrics side by side on any ledger.
//!
//! Usage: `cargo run --release --example audit -- <ledger.json>`
//!
//! This is how every number in the pre-launch audit was produced, and it is how
//! to check a scoring change against a real ledger rather than against intuition.

use anamnesis::model::Ledger;
use anamnesis::scoring::{self, Sample};

fn main() {
    let path = std::env::args().nth(1).expect("usage: audit <ledger.json>");
    let led: Ledger = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    let (mut first, mut last) = (Vec::new(), Vec::new());
    for c in &led.claims {
        if let (Some(o), Some(fp), Some(lp)) = (c.outcome(), c.first_prob(), c.current_prob()) {
            first.push(Sample::new(fp, o.happened()));
            last.push(Sample::new(lp, o.happened()));
        }
    }
    if last.is_empty() {
        println!("no resolved binary claims in {path}");
        return;
    }
    let corp = scoring::corp_brier(&last).unwrap();
    let floor = scoring::mcb_null_quantile(&last, 400, 0.95, 0xA11CE).unwrap();
    println!("n                    {}", last.len());
    println!(
        "brier (first fc)     {:.4}",
        scoring::brier(&first).unwrap()
    );
    println!("brier (final fc)     {:.4}", scoring::brier(&last).unwrap());
    println!(
        "exact-group REL      {:.4}",
        scoring::decompose(&last).unwrap().reliability
    );
    println!(
        "CORP  MCB            {:.4}   (noise floor q95 = {floor:.4})",
        corp.mcb
    );
    println!("CORP  DSC            {:.4}", corp.dsc);
    println!("CORP  UNC            {:.4}", corp.unc);
    println!(
        "identity residual    {:.3e}",
        (corp.score - (corp.mcb - corp.dsc + corp.unc)).abs()
    );
    println!(
        "e-value  shipped     {:.4}",
        scoring::calibration_eprocess(&last).unwrap()
    );
    println!(
        "e-value  proposed    {:.4e}  (log {:.2})",
        scoring::calibration_eprocess_v2(&last).unwrap(),
        scoring::calibration_log_eprocess(&last).unwrap()
    );
}
