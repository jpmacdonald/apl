use apl_schema::index::PackageIndex;
use std::path::Path;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = if args.len() > 1 {
        args[1].clone()
    } else {
        let home = std::env::var("HOME")?;
        format!("{home}/.apl/index")
    };

    let index = PackageIndex::load(Path::new(&path))?;
    println!("Index Version: {}", index.version);
    println!("Updated At: {}", index.updated_at);
    println!("Packages ({}):", index.packages.len());

    for p in &index.packages {
        println!(" - {}", p.name);
    }

    if let Some(aws) = index.find("aws-cli") {
        println!("Found aws-cli!");
        println!("Description: {}", aws.description);
    } else {
        println!("aws-cli not found in local index.");
    }

    Ok(())
}
