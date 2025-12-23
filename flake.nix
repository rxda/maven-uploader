{
  description = "System dependencies for Rust (OpenSSL)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          # 1. 编译辅助工具
          # pkg-config 是必须的，它帮助 cargo 找到 openssl 的具体位置
          nativeBuildInputs = with pkgs; [
            pkg-config
          ];

          # 2. 系统依赖库
          # 这里只放 Rust 项目依赖的 C 库
          buildInputs = with pkgs; [
            openssl
          ];

          # 3. 环境变量配置
          # 虽然 pkg-config 通常能搞定，但显式设置这些变量能解决大多数 edge case
          OPENSSL_DIR = "${pkgs.openssl.dev}";
          OPENSSL_LIB_DIR = "${pkgs.openssl.out}/lib";
          OPENSSL_INCLUDE_DIR = "${pkgs.openssl.dev}/include";

          # 4. 链接库路径
          # 帮助你的程序在运行时找到 .so 文件
          LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath [ pkgs.openssl ];

          shellHook = ''
            echo "🔧 System libraries loaded: OpenSSL"
            echo "   Rust toolchain: Managed by rustup (External)"
          '';
        };
      }
    );
}