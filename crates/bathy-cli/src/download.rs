use anyhow::{Context, Result, bail};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_BASE: &str = "https://downloads.rijkswaterstaatdata.nl/bodemhoogte_1mtr/";

/// File names ending in `.tif` listed on an Apache-style index page.
pub fn list_tifs(base: &str, suffix: &str) -> Result<Vec<String>> {
    let html = ureq::get(base).call()?.body_mut().read_to_string()?;
    let mut names: Vec<String> = html
        .split("href=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter(|h| h.ends_with(suffix) && !h.contains('/'))
        .map(str::to_owned)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Downloads `url` to `dest`, resuming a partial `.part` file and skipping complete files.
pub fn download_file(url: &str, dest: &Path) -> Result<()> {
    let remote_len: Option<u64> = ureq::head(url).call().ok().and_then(|r| r.headers().get("content-length")?.to_str().ok()?.parse().ok());
    if let (Some(len), Ok(meta)) = (remote_len, fs::metadata(dest)) {
        if meta.len() == len {
            println!("skip   {} (up to date)", dest.display());
            return Ok(());
        }
    }
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let mut req = ureq::get(url);
    if have > 0 {
        req = req.header("Range", format!("bytes={have}-"));
    }
    let resp = req.call().with_context(|| format!("GET {url}"))?;
    let resuming = have > 0 && resp.status().as_u16() == 206;
    let mut file = if resuming { OpenOptions::new().append(true).open(&part)? } else { File::create(&part)? };
    println!("fetch  {url}{}", if resuming { format!(" (resume at {have})") } else { String::new() });
    let mut body = resp.into_body();
    let mut reader = body.as_reader();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])?;
    }
    drop(file);
    if let Some(len) = remote_len {
        let got = fs::metadata(&part)?.len();
        if got != len {
            bail!("incomplete download of {url}: {got} of {len} bytes (run again to resume)");
        }
    }
    fs::rename(&part, dest)?;
    Ok(())
}

/// Appends missing `[[source]]` entries for the given GeoTIFFs to the registry file.
pub fn update_registry(registry: &Path, tifs: &[PathBuf]) -> Result<()> {
    let existing = fs::read_to_string(registry).unwrap_or_default();
    let base = registry.parent().unwrap_or(Path::new("."));
    let mut text = existing.clone();
    for tif in tifs {
        let rel = tif.strip_prefix(base).unwrap_or(tif).to_string_lossy().replace('\\', "/");
        if existing.contains(&format!("path = \"{rel}\"")) {
            continue;
        }
        let name = tif.file_stem().unwrap().to_string_lossy();
        text.push_str(&format!("\n[[source]]\nname = \"{name}\"\npath = \"{rel}\"\ncrs = \"EPSG:7415\"\npriority = 0\n"));
    }
    if text != existing {
        fs::write(registry, text)?;
    }
    Ok(())
}
