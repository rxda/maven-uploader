
# Maven Uploader CLI By Gemini

一个使用 Rust 编写的高性能、流式 Maven 构件批量上传工具。专门用于将本地 Maven 仓库（或备份目录）中的 `.jar` 和 `.pom` 文件批量迁移/上传至 Nexus 或 Artifactory 私服。

## ✨ 特性

- **高性能并行处理**：采用 `jwalk` 并行扫描文件系统，`rayon` 并行上传构件，配合 `crossbeam-channel` 实现“边扫描边上传”。
- **智能路径解析 (Smart GAV)**：不依赖脆弱的 POM XML 变量解析，而是基于文件在仓库中的**物理路径指纹**提取 GroupId、ArtifactId 和 Version，完美处理 `${revision}` 等变量。
- **增量上传 (SQLite 驱动)**：内置本地 SQLite 数据库记录上传状态，通过文件修改时间（mtime）判断，实现秒级二次启动与断点续传。
- **自动仓库分发**：自动识别 `-SNAPSHOT` 版本并将其分发到独立的快照仓库（需配置）。
- **灵活的过滤机制**：
  - **关键字过滤**：排除特定的 GroupId 或 ArtifactId。
  - **大小过滤**：自动跳过 Spring Boot Fat Jar 等超大运行包（默认 100MB）。
- **优雅的 UI**：底部固定进度条，日志在上方实时滚动，不闪烁。
- **NixOS 友好**：默认使用 `rustls`，无需在系统配置复杂的 OpenSSL 依赖。

## 🚀 安装

### 1. 源码编译 (Linux/macOS)
```bash
git clone <your-repo-url>
cd maven_uploader
cargo build --release
```

### 2. 交叉编译至 Windows (在 NixOS 下)
确保你的 `flake.nix` 包含 MinGW 工具链，然后运行：
```bash
rustup target add x86_64-pc-windows-gnu
cargo build --target x86_64-pc-windows-gnu --release
```

## 🛠 使用说明

### 命令行参数

| 参数 | 环境变量 | 说明 |
| :--- | :--- | :--- |
| `-U, --url` | `NEXUS_URL` | **(必填)** Release 仓库基础 URL |
| `-S, --snapshot-url`| `NEXUS_SNAPSHOT_URL` | Snapshot 仓库 URL (可选) |
| `-u, --username` | `NEXUS_USERNAME` | 仓库用户名 |
| `-p, --password` | `NEXUS_PASSWORD` | 仓库密码 |
| `-d, --dir` | `NEXUS_DIR` | 待扫描的根目录 (默认: `.`) |
| `-f, --force` | `NEXUS_FORCE` | 强制覆盖 (跳过本地 DB 和远程检查) |
| `-E, --exclude` | `NEXUS_EXCLUDE` | 排除关键字 (逗号分隔，如 `my-app,test`) |
| `--max-size` | - | 跳过大于此大小的文件 (MB, 默认: 100) |
| `--db-path` | - | 状态数据库路径 (默认: `uploader_state.db`) |

### 示例

#### 1. 使用环境变量 (推荐，配合 direnv)
在项目根目录创建 `.envrc`:
```bash
export NEXUS_URL="http://192.168.1.100:8081/repository/maven-releases/"
export NEXUS_SNAPSHOT_URL="http://192.168.1.100:8081/repository/maven-snapshots/"
export NEXUS_USERNAME="admin"
export NEXUS_PASSWORD="password123"
export NEXUS_EXCLUDE="internal-tool,temp-module"
```
执行 `direnv allow` 后直接运行：
```bash
./maven_uploader -d /path/to/local/repo
```

#### 2. 直接使用命令行参数
```bash
./maven_uploader -U http://nexus:8081/repository/maven-releases/ \
                 -u admin -p admin123 \
                 -d ./my-repo -E snapshot-old --max-size 50
```

## 📂 工作原理

1. **扫描阶段**：`jwalk` 多线程遍历目录寻找 `.pom` 文件。
2. **解析阶段**：
   - 提取文件相对于根目录的路径。
   - 倒数第 2 级为 `Version`，第 3 级为 `ArtifactId`，其余前缀为 `GroupId`。
   - 自动在同级目录下寻找对应的 `.jar`, `.war` 或 `.aar`。
3. **校验阶段**：
   - **L1 (Local)**: 检查 `uploader_state.db`，如果 URL 和文件修改时间匹配，直接跳过。
   - **L2 (Remote)**: 发送 HTTP HEAD 请求，若私服已存在，同步记录到本地 DB 并跳过。
4. **上传阶段**：使用 HTTP PUT 请求并行上传文件。

## ⚠️ 注意事项

- **目录结构**：请确保 `-d` 参数指定的目录是 Maven 仓库的“根”（即直接包含 `com/`, `org/` 的那一层）。如果路径层级不正确，生成的 GroupId 将会包含错误的目录前缀。
- **并发控制**：如果你的 Nexus 服务端响应缓慢或报 400 错误，请尝试设置环境变量 `export RAYON_NUM_THREADS=2` 降低并发。
- **数据库查看**：状态库为 SQLite 格式，你可以使用 `sqlite3 uploader_state.db` 进行手动查询。

## 📄 License
MIT 

---
*Created with Rust 🦀 for maximum performance.*