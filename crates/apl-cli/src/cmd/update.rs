//! Update command

use anyhow::{Context, Result, bail};
use apl_core::paths::apl_home;
use apl_schema::index::PackageIndex;
use reqwest::Client;

/// Update package index from CDN
pub async fn update(url: &str, upgrade_all: bool, dry_run: bool) -> Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};

    let index_path = apl_home().join("index");
    let output = crate::ui::Output::new();

    if dry_run {
        output.info(&format!("Would download index from: {url}"));
        output.info(&format!("Would save to: {}", index_path.display()));
        if upgrade_all {
            output.info("Would proceed to upgrade all packages.");
        }
        return Ok(());
    }

    let client = Client::new();
    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            output.error("Failed to check updates");
            return Err(e.into());
        }
    };

    if !response.status().is_success() {
        output.error(&format!("HTTP {}", response.status()));
        bail!("Failed to fetch index: HTTP {}", response.status());
    }

    let bytes = response.bytes().await?;

    // Verify signature
    if !dry_run {
        let sig_url = format!("{url}.sig");
        // output.info(&format!("Verifying signature: {}", sig_url)); // Optional verbosity

        let sig_response = client.get(&sig_url).send().await;

        match sig_response {
            Ok(resp) if resp.status().is_success() => {
                let sig_b64 = resp.text().await?.trim().to_string();

                let public_bytes = base64::engine::general_purpose::STANDARD
                    .decode(crate::APL_PUBLIC_KEY)
                    .unwrap(); // Static key must be valid
                let verifying_key =
                    VerifyingKey::from_bytes(public_bytes.as_slice().try_into().unwrap())
                        .map_err(|_| anyhow::anyhow!("Invalid public key length"))?;

                let signature_bytes = base64::engine::general_purpose::STANDARD
                    .decode(&sig_b64)
                    .context("Invalid Base64 signature")?;
                let signature = Signature::from_bytes(
                    signature_bytes
                        .as_slice()
                        .try_into()
                        .context("Invalid signature length")?,
                );

                if verifying_key.verify(&bytes, &signature).is_ok() {
                    // Signature valid; success message deferred until index is saved
                } else {
                    output.error("Signature verification FAILED");
                    bail!(
                        "Security Error: Index signature is invalid. This could be a MITM attack."
                    );
                }
            }
            _ => {
                // If signature is missing, decide policy.
                // For now, warn but allow (Trust on First Use / Transition Period) if user hasn't opted into strict mode?
                // Actually, the user approved "Hard Fail" in strict mode, but said "Modern".
                // Let's implement Strict Mode by default as per my plan ("Hard Fail").
                // If the signature file is missing, we fail.
                output.error("Missing index signature");
                bail!(
                    "Security Error: Index signature not found at {sig_url}. We enforce signed indexes."
                );
            }
        }
    }

    // Auto-detect ZSTD compression
    let decompressed = if bytes.len() >= 4 && bytes[0..4] == crate::ZSTD_MAGIC {
        zstd::decode_all(bytes.as_ref()).context("Failed to decompress index")?
    } else {
        bytes.to_vec()
    };

    let index = PackageIndex::from_bytes(&decompressed).context("Invalid index format")?;

    // Load current index for comparison
    let current_index = PackageIndex::load(&index_path).ok();

    if let Some(current) = current_index {
        if current.updated_at == index.updated_at {
            // "Index already up to date" is enough feedback
            output.success("Index already up to date");

            if upgrade_all {
                println!();
                return crate::cmd::upgrade::upgrade(&[], false, dry_run).await;
            }
            return Ok(());
        }
    }

    output.success("Index updated");

    // Save RAW (decompressed) data to disk for fast MMAP loading
    std::fs::write(&index_path, &decompressed)?;

    if upgrade_all {
        println!();
        return crate::cmd::upgrade::upgrade(&[], false, dry_run).await;
    }

    let db = crate::db::StateDb::open()?;
    let packages = db.list_packages()?;
    let mut update_list = Vec::new();

    for pkg in &packages {
        if let Some(entry) = index.find(&pkg.name) {
            let latest = match entry.latest() {
                Some(v) => v.version.clone(),
                None => continue,
            };
            if apl_schema::version::is_newer(&pkg.version, &latest) {
                update_list.push((pkg.name.clone(), pkg.version.clone(), latest));
            }
        }
    }

    if !update_list.is_empty() {
        use crossterm::style::Stylize;
        let theme = crate::ui::Theme::default();

        println!();
        for (name, old, new) in &update_list {
            let name_part = format!("{:<width$}", name, width = theme.layout.name_width);
            println!(
                "  {} {}  ->  {}",
                name_part.with(theme.colors.package_name),
                old.as_str().dark_grey(),
                new.as_str().with(theme.colors.success)
            );
        }
        println!();
        println!(
            "  {} upgrades available -- run 'apl upgrade' to apply",
            update_list.len()
        );
    }

    Ok(())
}
