# labjudge —— MatrOJ 局域网比赛系统

Rust 单体、服务端渲染、离线运行。学生用浏览器看题、提交 C++17；教师在 `/admin` 管理比赛、监控、榜单与导出。前端资源、模板全部内嵌进 exe，不需要 Node.js。

## 功能

- 比赛包扫描/校验：`contest.json`、`students.csv`、题面、测试点、子任务。
- 学生：账号 + 口令登录（账号由老师在名单里设定，学号或身份证号均可；口令由老师在 `students.csv` 的 `password` 列发放）、倒计时、比赛页三标签（题目列表 / 我的信息 / 排行榜）、全部题面、单题页、CodeMirror 草稿、提交后回题目页提示已提交。
- 开赛前：学生端只显示等待页并自动轮询，题面与题目列表一律不下发；开赛后页面自动刷新出现题目。比赛结束后学生端一律关闭，只显示“比赛已结束”，成绩由老师导出或线下告知。
- 判题：MinGW g++ 编译；go-judge 逐测试点运行；AC/WA/TLE/RE/CE；子任务 `sum`/`min`/`max`；OI 总分与排行榜。
- 可见性：榜单隐藏时学生只看到提交状态，不显示判定、分数、测试点、编译错误；公开后历史结果立即出现（仅限比赛进行中；结束后学生端关闭）。
- 教师：比赛页按「比赛信息与题目 / 实时监控 / 参赛者名单 / 排行榜」四个标签页组织（每块内容各占一张卡片）；开赛/结束、比赛进行中「延长比赛」、结束后「恢复比赛」（从现在起再开放若干分钟；恢复后学生需要重新登录）、榜单开关、全部重测、重置比赛（清空本场数据回到未开始）、系统信息；比赛结束后在「比赛信息与题目」页出现成绩 CSV 与源码 zip 导出。
- 监控与名单：实时监控分「评测器状态」（go-judge 存活、并发、队列深度、各状态计数）与「最近提交」两张卡片；参赛者名单按姓名列出谁真的登录进入了比赛、最近活动时间与提交数，未进入的排在最前。
- 安全：签名会话 Cookie、单账号单活动会话、比赛结束即封学生端、CSRF、路径穿越防护、隐藏模式集中裁剪；学生端编译输出只保留 `main.cpp` 诊断。

## 环境

- Windows 10/11 x64
- 构建：Rust 1.95.0、VS Build Tools 2022（MSVC）、`rust-toolchain.toml` 已固定
- 运行前置：`PATH` 上必须有可执行的 MinGW-w64 g++（**不随发布包分发**，也不读配置文件）
- 判题执行器：go-judge v1.14.1，固定存放在 `vendor/go-judge/go-judge.exe`（已提交）
- 无需 Node/npm/Vue/Vite/Tailwind；无前端构建步骤

go-judge 与 MathJax/CodeMirror 已固定入库：go-judge 在 `vendor/go-judge/`，前端资源在 `static/vendor/`。运行时不会下载任何文件，发布包也不含 C++ 编译器。

## 工具链要求

启动时只做一件事：执行 `g++ --version`。

- 执行成功且 `g++ -dumpmachine` 为 MinGW/Windows，才能继续；Cygwin/MSYS 的 g++ 会被拒绝（它们产出的 exe 依赖非系统 DLL，沙箱里跑不起来）。
- 执行失败（没装、不在 `PATH`、不是 MinGW）**程序直接报错退出**，不会启动 HTTP 服务。
- 不做任何目录扫描或路径配置兜底：`PATH` 是唯一来源。
- go-judge 同理：发布包的 `bin\go-judge.exe`（开发时回落 `vendor\go-judge\go-judge.exe`）缺失或无法启动时程序同样直接退出。

安装方式任选：MSYS2（`mingw-w64-ucrt-x86_64-gcc`）、winlibs 解压后把 `bin` 加进 `PATH`、Scoop `gcc`。装完在命令行确认 `g++ --version` 能跑。

## 快速开始（开发）

```powershell
cargo run
```

- 默认监听 `0.0.0.0:8080`，占用时顺延最多 10 个端口。
- 开发机同样需要 g++：探测不到会立即退出，不会起服务。
- 首次运行生成 `config.json`（12 位管理口令 + 会话密钥），口令打印在控制台。
- 开发/验收可用 `LABJUDGE_ROOT` 指向临时根目录。
- 健康检查：`GET /healthz`；静态资源：`/static/app.css`。

## 构建与发布

开发构建仍可直接 `cargo build --release`。生成可分发目录与 zip：

```powershell
cargo xtask package
```

`cargo xtask package` 会构建 release、校验固定 go-judge，并生成发布目录与 zip。发布包只有 `bin\` 下的两个 exe，不含 C++ 编译器。

发布目录 `dist\labjudge\`：

```text
start.bat                 # 启动脚本（双击运行）
bin\labjudge.exe
bin\go-judge.exe
contests\
  sample-contest\         # 示例比赛，启动即可在管理端看到
README.txt                # 面向教师的使用说明
```

`README.txt` 与 `start.bat` 的文案内嵌在 `xtask/src/package.rs` 里，不再复制仓库 README；发布前把其中的 `PROJECT_URL` 换成真实仓库地址即可。

同时生成 `dist\labjudge-YYYYMMDD.zip`。发布包不包含源码、`config.json`、`data/` 或 xtask。

## 教师机使用

1. 确认教师机 `PATH` 上有 MinGW g++：命令行执行 `g++ --version` 与 `g++ -dumpmachine`（输出应包含 `mingw`）。没有则先安装。
2. 解压发布包到可写目录，双击 `start.bat`（等价于运行 `bin\labjudge.exe`）。`g++` 或 `go-judge` 不可用时程序会直接报错退出，脚本会暂停窗口便于查看原因。
3. Windows 防火墙提示时只勾选“专用网络”并允许访问；不要把工具映射到公网。
4. 控制台打印局域网访问地址、管理地址与首次口令；首次启动自动打开 `/admin`。
5. `contests\sample-contest` 已随包提供，管理端开赛即可彩排；正式比赛把比赛包目录放进 `contests\` 后重启程序。
6. 杀毒软件若拦截新编译程序，将 `data\` 加入信任。
7. 全程离线；浏览器建议较新 Edge/Chrome。

程序按「`bin\` 的上一级」认定工具根目录，`config.json`、`data\`、`contests\` 都在根目录下。

## 比赛包

```text
contests/<比赛ID>/
├─ contest.json
├─ students.csv
└─ problems/A/
   ├─ problem.json
   ├─ statement.md
   ├─ images/...
   └─ testdata/1.in 1.out 2.in 2.out ...
```

- `students.csv` 列：`name`、`student_id`、`password`（必填，`school`、`grade` 可选，列顺序不限）。
  `student_id` 就是学生的登录账号，由老师任意设定（学号、身份证号都行），但必须唯一。
  `password` 由老师逐人填写或生成随机串后分发（打印清单、纸条等）；工具不提供口令查看入口。
  建议使用 8 位左右的随机字母数字；工具不做长度/复杂度校验，由老师负责。
  `student_id` 和 `password` 用 Excel 编辑时请把该列设为文本格式，否则前导零会被吃掉。
  名单里有重名不影响登录——账号唯一即可。
- 比赛 ID：`A-Za-z0-9` 开头，其后 `A-Za-z0-9._-`。
- 测试点按 basename 自然排序；每个 `.in` 必须有同名 `.out`。
- 题面只识别 `$...$` / `$$...$$` 公式；代码块内 `$` 不抽取。
- 非法比赛包只在教师端列出原因，学生端不展示。

## 判题与数据

- 每场比赛一个 SQLite：`data/<比赛ID>/contest.db`；源码永久保存，临时编译目录结束后清理。
- go-judge 只绑 `127.0.0.1`，随机端口 + 随机 Bearer token；启动失败即退出，运行中崩溃自动重启一次。
- 编译超时 10 秒；Job Object 限制进程树；输出比对统一 CRLF→LF、去行尾空白、去末尾空行。
- 榜单按每题最高分求和；总分相同按达成时间早者优先，再按账号。
- 备份：停止程序后复制 `data/<比赛ID>/`。
- 重置：管理端比赛详情页执行，删除该场提交与成绩，保留比赛包。

## 维护命令

- `cargo xtask package`：构建 release、校验 go-judge、生成发布目录与 zip（不含 g++）。
- `cargo xtask acceptance`：对发布包执行管理员/学生/真判题/隐藏/榜单/导出/重置验收。
- `cargo xtask acceptance --no-reset`：保留验收数据，便于重启持久化彩排。
- 压测：`$env:LABJUDGE_LOADTEST='1'; cargo test --test load_smoke -- --ignored --nocapture`

验收与发布命令依赖 Windows 自带的 `curl.exe`，仅在开发/维护机使用，不随发布包分发。

## 测试

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

真机判题测试：

```powershell
$env:LABJUDGE_REAL_JUDGE='1'; cargo test --test live_judge -- --ignored --nocapture --test-threads=1
```

构建目标为 `x86_64-pc-windows-msvc` + 静态 CRT；release exe 不依赖 `VCRUNTIME140.dll` / `MSVCP140.dll`。

## 安全与离线

- 仅局域网使用，不做 TLS；不要映射公网。
- 管理口令明文存 `config.json`（局域网威胁模型），可在 `/admin/system` 修改。
- 所有 POST 带 CSRF；Cookie 为 HttpOnly、SameSite=Lax。同源防护依赖 SameSite，不再声称校验 Origin/Referer。
- 同一账号只保留一个活动会话；比赛中重新登录会顶掉旧会话。比赛结束后学生会话失效，学生端完全关闭。
- 题面资源只允许 png/jpg/jpeg/gif/webp，canonicalize 后校验目录前缀。
- 模板与 `app.js`/`app.css` 不加载外部 CDN；运行时无外部网络请求。
- 双开由 `data/.lock` 阻止。
- **安全边界**：本工具面向受控教室，默认信任学生。Windows 版不提供代码沙箱；学生代码在教师机运行，可能读取教师机文件或联网；学生端的编译输出过滤只影响显示，不是隔离。只在教师可监督、学生可信的教室使用；高利害或不可信环境请改用带沙箱的 Linux 判题环境。

## 第三方版本与许可证

- go-judge v1.14.1（MIT）
  - 下载：`https://github.com/criyle/go-judge/releases/download/v1.14.1/go-judge_1.14.1_windows_amd64.exe`
  - 固定文件：`vendor/go-judge/go-judge.exe`（许可证 `vendor/go-judge/LICENSE`）
  - SHA256：`021bd390de60212c6875496bb4f1797a5d22693ef2459651856f1937d3309d3a`
- winlibs MinGW-w64 16.2.0 UCRT/SEH（GPL，含 GCC Runtime Library Exception）
  - 仅为开发机/教师机提供参考安装；发布包不分发、程序不下载，需使用者自行安装
  - 包：`winlibs-x86_64-posix-seh-gcc-16.2.0-mingw-w64ucrt-14.0.0-r3.zip`
  - SHA256：`3c69305081e4a647144339a936244fcf7d3d9e558319e1ad7be2cd2886d4b6a1`
- MathJax 3.2.2 tex-svg（Apache-2.0）
  - npm sha1：`c754d7b46a679d7f3fa03543d6b8bf124ddf9f6b`；许可证见 `static/vendor/mathjax/LICENSE`
- CodeMirror 5.65.16（MIT）
  - npm sha1：`efc0661be6bf4988a6a1c2fe6893294638cdb334`；许可证见 `static/vendor/codemirror/LICENSE`

本项目自身依赖 Rust/axum/rusqlite 等开源库，版本由 `Cargo.lock` 固定。

## 许可证

本项目以 MIT 许可证发布，全文见 [`LICENSE`](LICENSE)。上一节列出的第三方组件各自保留其原有许可证。
