{
  description = "Rust Multi-platform Build Environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        # 定义 Windows 交叉编译包集
        winPkgs = pkgs.pkgsCross.mingwW64;
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            pkg-config
            # 引入 Windows 交叉编译器，它在 Linux 下运行，但生成 Windows 代码
            winPkgs.stdenv.cc 
          ];

          buildInputs = with pkgs; [
            # Linux 原生依赖 (如果以后需要 OpenSSL 等)
            openssl 
          ];

          # --- 核心：隔离环境变量 ---

          # 1. 仅针对 Windows 目标的配置（不会影响 Linux 编译）
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "x86_64-w64-mingw32-gcc";
          
          # 2. 仅针对 Windows 目标的库路径（解决 lpthread 报错）
          # 注意变量名：CARGO_TARGET_<TARGET>_RUSTFLAGS
          CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS = "-L native=${winPkgs.windows.pthreads}/lib";

          # 3. 如果 Linux 编译也需要特定库，可以单独写
          # 例如：CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS = "...";

          shellHook = ''
          '';
        };
      }
    );
}