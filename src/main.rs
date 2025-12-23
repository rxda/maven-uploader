use anyhow::{Context, Result};
use clap::Parser;
use crossbeam_channel::unbounded;
use dashmap::DashMap;
use indicatif::{ProgressBar, ProgressStyle};
use jwalk::WalkDir;
use rayon::prelude::*;
use rusqlite::{params, Connection};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, UNIX_EPOCH};

/// 高性能 Maven 私服批量上传工具 (SQLite 持久化版)
#[derive(Parser, Debug, Clone)]
#[command(author, version, about)]
struct Args {
    /// Release 仓库 URL (-U)
    #[arg(short = 'U', long, env = "NEXUS_URL")]
    url: String,

    /// Snapshot 仓库 URL (-S)，不填则默认使用 Release URL
    #[arg(short = 'S', long, env = "NEXUS_SNAPSHOT_URL")]
    snapshot_url: Option<String>,

    /// 用户名 (-u)
    #[arg(short = 'u', long, env = "NEXUS_USERNAME")]
    username: String,

    /// 密码 (-p)
    #[arg(short = 'p', long, env = "NEXUS_PASSWORD")]
    password: String,

    /// 扫描根目录 (-d)，通常为包含 org/, com/ 的那一层
    #[arg(short = 'd', long, env = "NEXUS_DIR", default_value = ".")]
    dir: String,

    /// 是否强制覆盖上传（跳过数据库和远程检查）
    #[arg(short = 'f', long, env = "NEXUS_FORCE")]
    force: bool,

    /// 排除关键字，多个用逗号隔开 (如: -E my-app,test)
    #[arg(short = 'E', long, env = "NEXUS_EXCLUDE", value_delimiter = ',')]
    exclude: Vec<String>,

    /// 跳过大于此大小的文件 (MB)
    #[arg(long, default_value_t = 100)]
    max_size: u64,

    /// SQLite 状态数据库路径
    #[arg(long, default_value = "uploader_state.db")]
    db_path: String,
}

#[derive(Debug, Clone)]
struct MavenArtifact {
    group_id: String,
    artifact_id: String,
    version: String,
    pom_path: PathBuf,
    binary_path: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Arc::new(Args::parse());

    // 1. 初始化 SQLite 数据库
    let conn = Connection::open(&args.db_path).context("无法创建/打开 SQLite 数据库")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS uploads (url TEXT PRIMARY KEY, mtime INTEGER)",
        [],
    )?;
    let db = Arc::new(Mutex::new(conn));

    // 2. 规范化根目录
    let root_path = fs::canonicalize(&args.dir).with_context(|| format!("路径无效: {}", args.dir))?;

    // 3. 进度条设置 (固定在底部)
    let upload_pb = ProgressBar::new(0);
    upload_pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.blue} [{bar:40.cyan/blue}] {pos}/{len} {msg} ({percent}%)")?
            .progress_chars("#>-"),
    );

    let (tx, rx) = unbounded::<MavenArtifact>();
    let processed_paths = Arc::new(DashMap::new());

    // 4. 扫描线程 (生产者)
    let args_scanner = Arc::clone(&args);
    let pb_for_scan = upload_pb.clone();
    let processed_ref = Arc::clone(&processed_paths);
    let root_ref = root_path.clone();
    
    thread::spawn(move || {
        WalkDir::new(&args_scanner.dir)
            .parallelism(jwalk::Parallelism::RayonDefaultPool {
                busy_timeout: Duration::from_secs(1),
            })
            .into_iter()
            .filter_map(|e| e.ok())
            .for_each(|entry| {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                
                if name.ends_with(".pom") || name == "pom.xml" {
                    if let Ok(art) = extract_smart_gav(&path, &root_ref) {
                        if is_excluded(&art, &args_scanner, &pb_for_scan) { return; }
                        if processed_ref.insert(path.clone(), ()).is_none() {
                            pb_for_scan.inc_length(1);
                            let _ = tx.send(art);
                        }
                    }
                }
            });
    });

    // 5. 并行上传 (消费者)
    let client = Arc::new(
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .build()?,
    );
    
    // 限制上传并发数，保护私服稳定性
    rayon::ThreadPoolBuilder::new().num_threads(4).build_global().ok();

    rx.into_iter().par_bridge().for_each(|artifact| {
        let is_snapshot = artifact.version.ends_with("-SNAPSHOT");
        let raw_url = if is_snapshot {
            args.snapshot_url.as_ref().unwrap_or(&args.url)
        } else {
            &args.url
        };
        let base_url = if raw_url.ends_with('/') { raw_url.to_string() } else { format!("{}/", raw_url) };

        upload_pb.set_message(format!("{}:{}", artifact.artifact_id, artifact.version));
        
        let _ = process_artifact(&client, &base_url, &args, &artifact, &upload_pb, &db);
        
        upload_pb.inc(1);
    });

    upload_pb.finish_with_message("✅ 所有任务已完成");
    Ok(())
}

/// 路径驱动解析 GAV，解决 XML 变量及嵌套文件夹问题
fn extract_smart_gav(full_path: &Path, root_path: &Path) -> Result<MavenArtifact> {
    let abs_full_path = fs::canonicalize(full_path)?;
    let relative_path = abs_full_path.strip_prefix(root_path)
        .map_err(|_| anyhow::anyhow!("文件不在根目录下"))?;

    let components: Vec<String> = relative_path.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();

    if components.len() < 4 {
        return Err(anyhow::anyhow!("路径非 Maven 标准结构"));
    }

    let version = components[components.len() - 2].clone();
    let artifact_id = components[components.len() - 3].clone();
    let group_id = components[..components.len() - 3].join(".");

    // 修复生命周期的 XML packaging 读取
    let packaging = {
        if let Ok(content) = fs::read_to_string(full_path) {
            if let Ok(doc) = roxmltree::Document::parse(&content) {
                doc.root_element()
                    .children()
                    .find(|n| n.has_tag_name("packaging"))
                    .and_then(|n| n.text())
                    .map(|s| s.to_string())
            } else { None }
        } else { None }
    }.unwrap_or_else(|| "jar".to_string());

    let parent = full_path.parent().unwrap();
    let mut binary_path = None;
    for ext in [&packaging, "jar", "war", "aar", "tar.gz"] {
        let p = parent.join(format!("{}-{}.{}", artifact_id, version, ext));
        if p.exists() {
            binary_path = Some(p);
            break;
        }
    }

    Ok(MavenArtifact {
        group_id,
        artifact_id,
        version,
        pom_path: full_path.to_path_buf(),
        binary_path,
    })
}

fn is_excluded(art: &MavenArtifact, args: &Args, pb: &ProgressBar) -> bool {
    for pattern in &args.exclude {
        if art.artifact_id.contains(pattern) || art.group_id.contains(pattern) {
            pb.println(format!("  [🚫] 排除匹配 '{}': {}", pattern, art.artifact_id));
            return true;
        }
    }
    if let Some(bin_path) = &art.binary_path {
        if let Ok(meta) = fs::metadata(bin_path) {
            let size_mb = meta.len() / 1024 / 1024;
            if size_mb > args.max_size {
                pb.println(format!("  [🐘] 大文件过滤 ({}MB): {}", size_mb, art.artifact_id));
                return true;
            }
        }
    }
    false
}

fn process_artifact(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Args,
    artifact: &MavenArtifact,
    pb: &ProgressBar,
    db: &Arc<Mutex<Connection>>
) -> Result<()> {
    upload_file(client, base_url, args, artifact, &artifact.pom_path, "pom", pb, db)?;
    if let Some(bin_path) = &artifact.binary_path {
        let ext = bin_path.extension().and_then(|s| s.to_str()).unwrap_or("jar");
        upload_file(client, base_url, args, artifact, bin_path, ext, pb, db)?;
    }
    Ok(())
}

fn upload_file(
    client: &reqwest::blocking::Client,
    base_url: &str,
    args: &Args,
    artifact: &MavenArtifact,
    file_path: &Path,
    extension: &str,
    pb: &ProgressBar,
    db: &Arc<Mutex<Connection>>,
) -> Result<()> {
    let group_path = artifact.group_id.replace('.', "/");
    let file_name = format!("{}-{}.{}", artifact.artifact_id, artifact.version, extension);
    let target_url = format!("{}{}/{}/{}/{}", base_url, group_path, artifact.artifact_id, artifact.version, file_name);

    let mtime = fs::metadata(file_path)?.modified()?.duration_since(UNIX_EPOCH)?.as_secs() as i64;

    if !args.force {
        // 1. 本地数据库检查 (SQLite)
        let skip_db = {
            let conn = db.lock().unwrap();
            let mut stmt = conn.prepare("SELECT mtime FROM uploads WHERE url = ?")?;
            let mut rows = stmt.query(params![target_url])?;
            if let Some(row) = rows.next()? {
                let recorded_mtime: i64 = row.get(0)?;
                recorded_mtime == mtime
            } else { false }
        };
        if skip_db {
            pb.println(format!("  [-] 远程(DB)已存在: {}", file_name));
            return Ok(()); 
        }

        // 2. 远程 HEAD 检查
        let resp = client.head(&target_url).basic_auth(&args.username, Some(&args.password)).send();
        if let Ok(r) = resp {
            if r.status().is_success() {
                save_status(db, &target_url, mtime)?;
                pb.println(format!("  [-] 远程已存在: {}", file_name));
                return Ok(());
            }
        }
    }

    // 3. 执行上传
    let file_data = fs::read(file_path)?;
    let put_resp = client.put(&target_url)
        .basic_auth(&args.username, Some(&args.password))
        .body(file_data)
        .send();

    match put_resp {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                pb.println(format!("  [+] 上传成功: {}", file_name));
                save_status(db, &target_url, mtime)?;
            } else {
                let msg = resp.text().unwrap_or_else(|_| "响应体解析失败".into());
                pb.println(format!("  [❌] 失败 ({}): {} - {}", status, file_name, msg));
            }
        }
        Err(e) => pb.println(format!("  [❌] 网络错误: {} - {}", file_name, e)),
    }

    Ok(())
}

fn save_status(db: &Arc<Mutex<Connection>>, url: &str, mtime: i64) -> Result<()> {
    let conn = db.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO uploads (url, mtime) VALUES (?, ?)",
        params![url, mtime],
    )?;
    Ok(())
}