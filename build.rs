fn main() -> Result<(), Box<dyn std::error::Error>> {
    let protos = [
        "common",
        "instruments",
        "marketdata",
        "operations",
        "orders",
        "sandbox",
        "stoporders",
        "users",
    ];
    let protos = protos
        .into_iter()
        .map(|e| format!("tinkoff_api/src/docs/contracts/{}.proto", e))
        .collect::<Vec<_>>();
    tonic_build::configure().compile(&protos, &["tinkoff_api/src/docs/contracts"])?;
    Ok(())
}
