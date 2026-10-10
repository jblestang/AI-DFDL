//! `dfdl-cli`: Command-line tool executable for DFDL parsing, unparsing, and schema compilation.

fn main() {
    println!("dfdl-cli v0.1.0 — DFDL Engine CLI Wrapper");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies CLI entrypoint execution and basic banner display.
    #[test]
    fn test_cli_entrypoint() {
        main();
    }
}

