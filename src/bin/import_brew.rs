//! Tool to import Homebrew packages into APL
//! Usage: cargo run --bin import_brew -- <package_names>...

use anyhow::{Context, Result};
use apl::core::package::{
    Dependencies, Hints, InstallSpec, Package, PackageInfo, PackageType, Source,
};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("Usage: cargo run --bin import_brew -- <package_names>...");
        return Ok(());
    }

    let client = reqwest::Client::new();
    let packages_dir = std::env::current_dir()?.join("packages");
    fs::create_dir_all(&packages_dir)?;

    for name in args {
        println!("Fetching info for {}...", name);
        if let Err(e) = import_package(&client, &name, &packages_dir).await {
            eprintln!("Exclude {}: {}", name, e);
        }
    }

    Ok(())
}

async fn import_package(client: &reqwest::Client, name: &str, out_dir: &PathBuf) -> Result<()> {
    let url = format!("https://formulae.brew.sh/api/formula/{}.json", name);
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("Brew API returned {}", resp.status());
    }

    let json: serde_json::Value = resp.json().await?;

    let version = json["versions"]["stable"]
        .as_str()
        .context("No stable version found")?
        .to_string();

    let desc = json["desc"].as_str().unwrap_or("").to_string();
    let homepage = json["homepage"].as_str().unwrap_or("").to_string();
    let _license = json["license"].as_str().unwrap_or("").to_string(); // Simplify license for now as string

    // Source URL (usually tarball)
    let src_url = json["urls"]["stable"]["url"]
        .as_str()
        .map(|s| s.to_string())
        .unwrap_or_default();

    // Brew doesn't always expose easy blake3/sha256 for source in the main JSON in a uniform way,
    // often it's sha256. APL uses blake3.
    // For now, we'll leave blake3 empty and let 'apl update' fill it, OR fetch and hash it?
    // 'apl update' is designed to fill missing hashes if we have the URL.
    // So we can leave it empty.

    // Binaries (Bottles)
    // Brew bottles are technically relocatable but often rely on specific paths.
    // APL prefers static binaries or compiling from source.
    // Ideally we want to find "bottles" that work on macOS arm64/x86_64.

    // Construct Package struct
    let pkg_info = PackageInfo {
        name: name.to_string(),
        version: version.clone(),
        description: desc,
        homepage,
        license: "MIT".to_string(), // Placeholder, parsing SPDX is hard
        type_: PackageType::Cli,
    };

    // Dependencies
    let deps = json["dependencies"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_else(Vec::new);

    let build_deps = json["build_dependencies"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| v.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .unwrap_or_else(Vec::new);

    let package = Package {
        package: pkg_info,
        source: Source {
            url: src_url,
            blake3: String::new(), // To be filled by update
            strip_components: 1,
        },
        binary: HashMap::new(), // We'll skip bottles for now and force source build or manual binary addition
        dependencies: Dependencies {
            runtime: deps,
            build: build_deps.clone(),
            optional: vec![],
        },
        install: InstallSpec {
            bin: vec![name.to_string()], // Guess binary name matches package
            lib: vec![],
            include: vec![],
            script: String::new(),
            app: None,
        },
        hints: Hints {
            post_install: String::new(),
        },
        build: Some(apl::core::package::BuildSpec {
            dependencies: vec![], // We merge build_deps into main deps for now in struct, but splitting is better
            script: if build_deps.contains(&"rust".to_string()) {
                "cargo build --release".to_string()
            } else if build_deps.contains(&"cmake".to_string()) {
                "cmake . -DCMAKE_BUILD_TYPE=Release && make".to_string()
            } else if build_deps.contains(&"go".to_string()) {
                "go build -ldflags='-s -w'".to_string()
            } else {
                "make".to_string() // Default to make, or "TODO"
            },
        }),
    };

    // Save to packages/name.toml
    let toml = toml::to_string_pretty(&package)?;
    let dest = out_dir.join(format!("{}.toml", name));
    fs::write(&dest, toml)?;
    println!("Saved {}", dest.display());

    Ok(())
}
