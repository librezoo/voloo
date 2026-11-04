# Voloo Host 开发约定

- 本仓库是 USB-C 版 Voloo Switch（CH32X035，USB CDC 点对点）的本机主机软件：
  单个 Rust 程序 `voloo`，提供 CLI、MCP、本机 HTTP/SSE 和网页面板。
- 不支持旧 RS-485 板，不做局域网访问。Voloo 只负责通断，不是电源；不能把 OFF 说成端子无电压。
- 协议以固件仓库中的主机协议 v2.5 文档为准（测试向量在 `tests/fixtures/protocol-examples.json`）。必须保留：完整 UID 定向、命令后回读确认、
  丢 ACK 不重发、结果未知时要求先查状态、等待超时不取消设备动作、测量无效返回 null 而不是 0。
- 只维护一份 Rust 实现。其他语言通过 HTTP 接口、CLI `--json` 或 MCP 接入，不新增各语言 SDK 的业务逻辑。
- 常规测试只用离线、模拟设备和 PTY。实板测试必须当次确认目标板、接线和供电；已有记录不能冒充本版实测。
- 提交小步、使用 Conventional Commits。不提交本机日志、诊断包或令牌。构建产物放在忽略目录。
