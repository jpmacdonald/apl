use apl::core::index::PackageIndex;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let index = PackageIndex::load(Path::new("index.bin"))?;
    if let Some(entry) = index.find("lua") {
        println!("{:#?}", entry);
    } else {
        println!("lua not found in index");
    }
    Ok(())
}
