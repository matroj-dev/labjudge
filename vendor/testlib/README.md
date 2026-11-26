# testlib

- 版本：0.9.52（`#define VERSION "0.9.52"`）
- 来源：<https://github.com/MikeMirzayanov/testlib>
- 固定 commit：`93214ef6a94e3669f6a8ec452754169775eb5b07`
- SHA256：`70683174e7e4d99a5ee46d47aa4fa2d12fc85631443181a12a2e5023152397f0`
- 许可证：MIT（见文件头部版权声明）

用途：编译题目目录下的 `checker.cpp` 时作为唯一的 `#include "testlib.h"` 来源。
`xtask package` 把它复制到发布包的 `include/testlib.h` 并校验 SHA；升级版本时同步更新
`xtask/src/package.rs` 的 `TESTLIB_SHA` 与本文件。
