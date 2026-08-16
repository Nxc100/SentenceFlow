//! `sf-license` — vendor CLI for key generation and license issuing.
//!
//! 目标:爱发电订单 → 一条命令签发 → 邮件发出,每单 <10 秒 (spec §7.6)。
//!
//! ```text
//! sf-license keygen --out-dir ./keys
//! sf-license issue --email x@y.com --major-max 3 --key-file ./keys/sf-license-private.secret
//! ```

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sf_license::issuer;
use sf_license::{LicensePayload, PRODUCT_ID};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(
    name = "sf-license",
    about = "SentenceFlow offline license issuer (vendor only)"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Generate a new Ed25519 keypair. Keep the private key OFFLINE.
    Keygen {
        /// Directory to write `sf-license-private.secret` / `sf-license-public.txt`.
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
    },
    /// Issue a `.sflic` license for one customer.
    Issue {
        /// Customer email (goes into the license, shown masked in-app).
        #[arg(long)]
        email: String,
        /// Highest covered product major version.
        #[arg(long, default_value_t = 1)]
        major_max: u32,
        /// License edition label.
        #[arg(long, default_value = "personal")]
        edition: String,
        /// Path to the base64 private key file.
        #[arg(long)]
        key_file: PathBuf,
        /// Output path; defaults to `<email>.sflic` in the current directory.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { out_dir } => keygen(&out_dir),
        Cmd::Issue {
            email,
            major_max,
            edition,
            key_file,
            out,
        } => issue(&email, major_max, &edition, &key_file, out),
    }
}

fn keygen(out_dir: &PathBuf) -> Result<()> {
    std::fs::create_dir_all(out_dir)?;
    let private_path = out_dir.join("sf-license-private.secret");
    if private_path.exists() {
        bail!(
            "{} already exists — refusing to overwrite a signing key",
            private_path.display()
        );
    }
    let (private_b64, public_b64) = issuer::generate_keypair();
    std::fs::write(&private_path, &private_b64)?;
    std::fs::write(out_dir.join("sf-license-public.txt"), &public_b64)?;
    println!("private key : {}", private_path.display());
    println!("public key  : {public_b64}");
    println!();
    println!("→ 把公钥粘贴进 apps/desktop 的 LICENSE_PUBLIC_KEY 常量;");
    println!("→ 私钥文件立刻转移到离线 U 盘 + 密码管理器,绝不入仓库。");
    Ok(())
}

fn issue(
    email: &str,
    major_max: u32,
    edition: &str,
    key_file: &PathBuf,
    out: Option<PathBuf>,
) -> Result<()> {
    if !email.contains('@') {
        bail!("'{email}' 不是有效邮箱");
    }
    let key_b64 = std::fs::read_to_string(key_file)
        .with_context(|| format!("读取私钥失败: {}", key_file.display()))?;
    let sk = issuer::parse_private_key(&key_b64).map_err(|e| anyhow::anyhow!(e))?;
    let payload = LicensePayload {
        v: sf_license::FORMAT_VERSION,
        product: PRODUCT_ID.into(),
        edition: edition.into(),
        email: email.into(),
        major_max,
        issued_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64,
    };
    let doc = issuer::issue(&payload, &sk);

    // Self-check before handing anything to a customer.
    let pk = issuer::public_key_of(&sk);
    sf_license::verify(&doc, &pk).map_err(|e| anyhow::anyhow!("自检失败: {e}"))?;

    let out = out.unwrap_or_else(|| PathBuf::from(format!("{}.sflic", email.replace('@', "_at_"))));
    std::fs::write(&out, &doc)?;
    println!("issued → {}", out.display());
    Ok(())
}
