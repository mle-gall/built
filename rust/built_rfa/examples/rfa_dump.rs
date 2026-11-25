//! Debug: `cargo run --example rfa_dump -- FILE.rfa [OUT.json]` prints a summary
//! and writes the full `built.rfa/1` JSON when OUT is given.
fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let v = built_rfa::read_family(&args[0])?;
    println!("family {}", v["family"]);
    for t in v["types"].as_array().unwrap() {
        println!(
            "type {:?} source {} components {}",
            t["name"].as_str().unwrap_or(""),
            t["geometry_source"],
            t["components"]
        );
    }
    for (id, g) in v["geometries"].as_object().unwrap() {
        let parts = g["parts"].as_array().unwrap();
        let tris: usize = parts
            .iter()
            .map(|p| p["indices"].as_array().unwrap().len() / 3)
            .sum();
        let hidden = parts.iter().filter(|p| p["hidden"] == true).count();
        let zones = parts.iter().filter(|p| p["zone"] == true).count();
        println!(
            "geometry {id} {:?}: {} parts, {tris} tris, {hidden} hidden, {zones} zone",
            g["label"].as_str().unwrap_or(""),
            parts.len()
        );
    }
    println!("materials {}", v["materials"]);
    println!("styles {}", v["styles"]);
    println!("warnings {}", v["warnings"]);
    if let Some(out) = args.get(1) {
        std::fs::write(out, serde_json::to_string(&v)?)?;
    }
    Ok(())
}
