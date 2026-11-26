// SPJ 示例：浮点容差比较。
//
// 协议（testlib 0.9.52）：
//   checker.exe <输入> <选手输出> <标准答案>
//   退出码 0 通过；1/2/4 判错；3 表示题目或数据有问题（整份提交会记为「判题失败」）。
//
// 编译：系统在加载比赛包时自动编译本文件，你需要保证它是一个能独立编译的单文件。
#include "testlib.h"

int main(int argc, char* argv[]) {
    registerTestlibCmd(argc, argv);

    // inf = 输入，ouf = 选手输出，ans = 标准答案
    int r = inf.readInt(1, 1000, "r");
    double expected = ans.readDouble();
    double got = ouf.readDouble();

    double area = 3.14159265358979323846 * (double) r * (double) r;
    if (!doubleCompare(expected, area, 1e-6)) {
        quitf(_fail, "标准答案与输入不一致：r=%d，答案应为 %.10f，实际 %.10f", r, area, expected);
    }

    if (!doubleCompare(got, expected, 1e-6)) {
        quitf(_wa, "答案错误：期望 %.10f，实际 %.10f", expected, got);
    }

    quitf(_ok, "面积 %.10f", got);
}
