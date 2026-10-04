fn main() -> anyhow::Result<()> {
    if std::env::args().skip(1).any(|a| a == "--version" || a == "-V") {
        println!("{}", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}
