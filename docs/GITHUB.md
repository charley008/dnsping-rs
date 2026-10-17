# 上传到 GitHub

本地 Git 仓库已初始化，主分支为 `main`。`target/`、`dist/`、日志和本地环境文件不会提交；`Cargo.lock`、图标、截图和完整源码保留在版本管理中。

## 首次推送

1. 在 GitHub 创建一个空仓库。不要同时勾选创建 README、许可证或 .gitignore，避免与现有初始提交产生不同历史。
2. 在本项目目录执行以下命令，将示例 URL 换成你的仓库地址：

```powershell
git remote add origin https://github.com/YOUR_USERNAME/YOUR_REPOSITORY.git
git push -u origin main
```

如已配置远程地址，先用 `git remote -v` 查看；需要更换地址时使用：

```powershell
git remote set-url origin https://github.com/YOUR_USERNAME/YOUR_REPOSITORY.git
```

## 后续修改

```powershell
git status
git add .
git diff --cached
git commit -m "Describe your changes"
git push
```

## 标签自动发布 Windows 程序

`.github/workflows/release.yml` **只监听 `v` 加数字开头的标签推送**；普通代码推送、PR 都不编译。推送标签后，GitHub 的 Windows Runner 会执行格式检查、测试、Clippy、Release 编译和打包，成功后自动创建 GitHub Release 并上传便携 ZIP、独立 exe、源码 ZIP、校验文件。无需另设 Token，使用仓库自带的 `GITHUB_TOKEN`。

首次版本是 `v0.1.0`，与 `Cargo.toml` 的 `version = "0.1.0"` 一致。操作如下：

```powershell
git tag -a v0.1.0 -m "DNSPing 0.1.0"
git push origin v0.1.0
```

标签应指向已验证并推送的代码版本，且必须与 Cargo 包版本一致。未来发布例如 `v0.1.1` 时，先更新 Cargo.toml 版本并执行 `cargo check` 更新 Cargo.lock，提交推送后再创建对应标签。

标签构建的是该标签指向的代码，不会意外打包随后才提交的 main 分支内容。可在仓库 Actions 查看进度；所有检查成功后，Release 才会公开。失败的运行可在 Actions 中重试。

另一台没有编译环境的电脑打开仓库的 [Releases 页面](https://github.com/charley008/dnsping-rs/releases/latest)，下载 `DNSPing-windows-x64.zip`。关闭程序后解压覆盖原目录即可；实际配置 `settings.json` 不在发行包里，会保留在程序目录。

本机也可以执行 `scripts/package.ps1` 手动打包，产物同样在 `dist/`，不需要把产物加入 Git。
