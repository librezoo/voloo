# USB 固件升级与回滚

本教程用于已支持在线升级的 USB-C Voloo Switch。主机如果提示设备或升级包不兼容，
请保留报告并联系支持，不要通过擦除设备或修改保护设置绕过检查。

## 准备

安装本版主机：

```sh
npm install -g @librezoo/voloo@0.7.2
voloo ls
```

如果旧版后台还在运行，先等所有设备操作完成，再执行一次 `voloo daemon stop`，
随后用新版 `voloo ls` 启动后台，并核对 `voloo daemon status` 显示 0.7.2。
这是更换主机程序的步骤；普通固件升级期间不要停止后台。

## 升级

将 `DEVICE_UID` 替换为列表中的完整 UID：

```sh
voloo status DEVICE_UID
voloo firmware status DEVICE_UID
voloo firmware update DEVICE_UID
```

开始前设备必须空闲、处于 OFF，VIN/VOUT 测量有效且 VOUT ≤ 1 V。OFF 本身不代表
端子电压为零；需要时先单独执行泄放并确认结果。升级不会替你关闭、泄放或恢复 ON。
设备操作锁仍然生效。

命令自动下载与当前主机版本配对的加密固件，不会搜索任意更新版本。升级期间保持
USB 供电稳定，不要停止后台。后台管理端口占用和重连；第三方串口程序应先退出。

升级后检查：

```sh
voloo firmware status DEVICE_UID
voloo info DEVICE_UID
voloo status DEVICE_UID
```

确认版本、原昵称和 OFF 状态符合预期。结果未知时先读取状态，不要盲目重发。

## 本地文件与网页

从对应官方 Release 下载推荐的 `.voloofw` 文件：

```sh
voloo firmware inspect ./voloo-switch-x035.voloofw
voloo firmware update DEVICE_UID ./voloo-switch-x035.voloofw
```

`inspect` 只做离线结构检查，不接触设备，也不能替代设备认证。在 `voloo ui` 的
设备详情中，可使用“USB 固件”的在线升级按钮，或选择匹配的本地文件。
临时下载文件在成功或正常处理失败后清理。

## 回滚与故障

```sh
voloo firmware rollback DEVICE_UID
```

回滚要求存在可用的上一版本，且设备状态和测量满足要求。新上传可能覆盖上一镜像。
试运行失败时，系统在可恢复条件下返回此前确认的版本；物理损坏不属于普通回滚能力。
持续失联、不兼容或没有可用上一版本时，请保留诊断报告并联系支持，不要整片擦除。

普通用户无需额外烧录器。更多行为与验证范围见[英文说明](firmware-updates.md)。
