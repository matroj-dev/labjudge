# Vendored Frontend Assets

本目录文件随 `labjudge.exe` 内嵌，运行时只从本机加载，不请求 CDN。

## MathJax 3.2.2

- 文件：`mathjax/tex-svg.js`、`mathjax/LICENSE`
- 来源：`https://registry.npmjs.org/mathjax/-/mathjax-3.2.2.tgz`
- npm tarball sha1：`c754d7b46a679d7f3fa03543d6b8bf124ddf9f6b`
- 许可证：Apache License 2.0（见 `mathjax/LICENSE`）
- 用途：浏览器端将 `\(...\)` / `\[...\]` 排版为 SVG。
- 未修改源文件；只提取 `package/es5/tex-svg.js` 与 `package/LICENSE`。

## CodeMirror 5.65.16

- 文件：`codemirror/codemirror.js`、`codemirror/codemirror.css`、`codemirror/clike.js`、`codemirror/LICENSE`
- 来源：`https://registry.npmjs.org/codemirror/-/codemirror-5.65.16.tgz`
- npm tarball sha1：`efc0661be6bf4988a6a1c2fe6893294638cdb334`
- 许可证：MIT（见 `codemirror/LICENSE`）
- 用途：C++ 编辑器、行号、C++ 高亮。
- 未修改源文件；只提取 `lib/codemirror.js`、`lib/codemirror.css`、`mode/clike/clike.js`、`LICENSE`。

## 校验

逐个文件的 SHA256 见 `SHA256SUMS.txt`。发布前可用以下命令复核：

```powershell
# 在仓库根目录执行
Get-FileHash -Algorithm SHA256 -Path '.\static\vendor\mathjax\tex-svg.js','.\static\vendor\codemirror\codemirror.js'
```
