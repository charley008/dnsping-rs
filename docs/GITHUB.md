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

## 发布 Windows 程序

先运行 `scripts/package.ps1` 生成最新发行文件，再创建一个 GitHub Release，上传 `dist/DNSPing-windows-x64.zip`；也可以附上 `DNSPing.exe` 与 `SHA256SUMS.txt`。这些文件无需强制加入 Git。

需要创建版本标签时：

```powershell
git tag -a v0.1.0 -m "DNSPing 0.1.0"
git push origin v0.1.0
```

标签应指向已验证并推送的代码版本；版本号随后续发布更新。
