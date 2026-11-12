// Small, dependency-free locale catalog. Device names, IDs and raw API details stay untouched.
'use strict';
(() => {
const messages = {
  "Invalid device state response": ["设备状态响应无效", "裝置狀態回應無效"],
  "Devices": [
    "设备",
    "裝置"
  ],
  "Activity": [
    "活动记录",
    "活動紀錄"
  ],
  "View": [
    "视图",
    "檢視"
  ],
  "Search": [
    "搜索",
    "搜尋"
  ],
  "Search devices": [
    "搜索名称或 UID",
    "搜尋名稱或 UID"
  ],
  "Summary": [
    "设备概况",
    "裝置概況"
  ],
  "Show": [
    "显示",
    "顯示"
  ],
  "All": [
    "全部",
    "全部"
  ],
  "Faults": [
    "故障",
    "故障"
  ],
  "Connect a Voloo": [
    "连接你的 Voloo",
    "連接你的 Voloo"
  ],
  "Plug it in with a USB data cable — it appears here within a second.": [
    "用 USB 数据线连接，设备就会自动显示。",
    "用 USB 資料線連接，裝置就會自動顯示。"
  ],
  "Nothing showing up? Run": [
    "没有显示？运行",
    "沒有顯示？執行"
  ],
  "No hardware at hand? Try": [
    "手边没有硬件？试试",
    "手邊沒有硬體？試試"
  ],
  "No matching devices": [
    "没有匹配的设备",
    "沒有符合的裝置"
  ],
  "Try another name or UID, or clear the search.": [
    "换个名称或 UID 试试，或清空搜索。",
    "換個名稱或 UID 試試，或清空搜尋。"
  ],
  "Voloo switches external power — it is not a power source. Off does not mean 0 V at the load; Discharge confirms VOUT ≤ 1 V. “—” means a reading is invalid, never zero.": [
    "Voloo 控制外部供电的通断，本身不提供电源。关闭不代表负载端为 0 V；泄放会确认 VOUT ≤ 1 V。“—”表示读数无效，不是零。",
    "Voloo 控制外部供電的通斷，本身不提供電源。關閉不代表負載端為 0 V；洩放會確認 VOUT ≤ 1 V。「—」表示讀數無效，不是零。"
  ],
  "Connecting": [
    "正在连接",
    "正在連線"
  ],
  "Reconnecting": [
    "正在重新连接",
    "正在重新連線"
  ],
  "Live · LibreZoo": [
    "实时连接 · LibreZoo",
    "即時連線 · LibreZoo"
  ],
  "Signed out — run voloo ui": [
    "会话已退出，请运行 voloo ui",
    "工作階段已登出，請執行 voloo ui"
  ],
  "Signed out": [
    "会话已退出",
    "已登出"
  ],
  "This page is signed out. Run `voloo ui` again.": [
    "会话已退出，请重新运行 `voloo ui`。",
    "工作階段已登出，請重新執行 `voloo ui`。"
  ],
  "now": [
    "刚刚",
    "剛剛"
  ],
  "Today": [
    "今天",
    "今天"
  ],
  "Yesterday": [
    "昨天",
    "昨天"
  ],
  "Offline": [
    "离线",
    "離線"
  ],
  "On": [
    "已开启",
    "已開啟"
  ],
  "Off": [
    "已关闭",
    "已關閉"
  ],
  "Switching on": [
    "正在开启",
    "正在開啟"
  ],
  "Switching off": [
    "正在关闭",
    "正在關閉"
  ],
  "Discharging": [
    "正在泄放",
    "正在洩放"
  ],
  "Cycling": [
    "正在断电重启",
    "正在斷電重啟"
  ],
  "Renaming": [
    "正在重命名",
    "正在重新命名"
  ],
  "Working": [
    "处理中",
    "處理中"
  ],
  "Busy": [
    "忙碌",
    "忙碌"
  ],
  "Check state": [
    "读取状态",
    "讀取狀態"
  ],
  "Fault": [
    "故障",
    "故障"
  ],
  "Not connected": [
    "未连接",
    "未連線"
  ],
  "Hold to switch off": [
    "长按关闭",
    "長按關閉"
  ],
  "Hold to switch on": [
    "长按开启",
    "長按開啟"
  ],
  "Keep holding…": [
    "请继续按住…",
    "請繼續按住…"
  ],
  "Switch On": [
    "开启供电",
    "開啟供電"
  ],
  "Switch Off": [
    "关闭供电",
    "關閉供電"
  ],
  "Hold to confirm": [
    "长按确认",
    "長按確認"
  ],
  "Discharge": [
    "主动泄放",
    "主動洩放"
  ],
  "Off, then drain ≤ 1 V": [
    "关闭并泄放至 ≤ 1 V",
    "關閉並洩放至 ≤ 1 V"
  ],
  "Power Cycle": [
    "断电重启",
    "斷電重啟"
  ],
  "Power cycle": [
    "断电重启设置",
    "斷電重啟設定"
  ],
  "Off for": [
    "断电时长",
    "斷電時長"
  ],
  "Discharge before waiting": [
    "先泄放再计时",
    "先洩放再計時"
  ],
  "Wait only once VOUT ≤ 1 V is confirmed": [
    "确认 VOUT ≤ 1 V 后才开始等待",
    "確認 VOUT ≤ 1 V 後才開始等待"
  ],
  "Allow changes": [
    "允许操作",
    "允許操作"
  ],
  "Needs attention": [
    "需要关注",
    "需要留意"
  ],
  "Recent activity": [
    "最近活动",
    "最近活動"
  ],
  "Access": [
    "操作权限",
    "操作權限"
  ],
  "Device": [
    "设备信息",
    "裝置資訊"
  ],
  "Name": [
    "设备名称",
    "裝置名稱"
  ],
  "Close": [
    "关闭详情",
    "關閉詳情"
  ],
  "Save": [
    "保存",
    "儲存"
  ],
  "Cancel": [
    "取消",
    "取消"
  ],
  "Click to rename": [
    "点击修改名称",
    "點選修改名稱"
  ],
  "Unlock to rename": [
    "解锁后才能修改名称",
    "解鎖後才能修改名稱"
  ],
  "Switch off to rename": [
    "关闭供电后才能修改名称",
    "關閉供電後才能修改名稱"
  ],
  "Locked": [
    "已锁定",
    "已鎖定"
  ],
  "Locked — open to unlock": [
    "已锁定，打开详情解锁",
    "已鎖定，開啟詳情解鎖"
  ],
  "Locked in config.toml": [
    "已在 config.toml 中锁定",
    "已在 config.toml 中鎖定"
  ],
  "Locked — no app, script or agent can switch or rename it": [
    "已锁定，应用、脚本和 AI 均无法切换供电或改名",
    "已鎖定，應用程式、腳本和 AI 均無法切換供電或改名"
  ],
  "Web, Terminal, AI agents and the API can switch it": [
    "网页、终端、AI 和 API 均可操作",
    "網頁、終端機、AI 和 API 均可操作"
  ],
  "No app, script or agent can switch it until you unlock it.": [
    "解锁前，所有应用、脚本和 AI 都无法切换供电。",
    "解鎖前，所有應用程式、腳本和 AI 都無法切換供電。"
  ],
  "Couldn't change the lock": [
    "无法修改锁定状态",
    "無法修改鎖定狀態"
  ],
  "Could not read device state": [
    "无法读取设备状态",
    "無法讀取裝置狀態"
  ],
  "Connection lost. Read the device state before trying another action.": [
    "连接中断。请先读取设备状态，再执行其他操作。",
    "連線中斷。請先讀取裝置狀態，再執行其他操作。"
  ],
  "Result unknown — check before acting": [
    "结果未知，请先读取状态",
    "結果未知，請先讀取狀態"
  ],
  "Result unknown. ": [
    "结果未知。",
    "結果未知。"
  ],
  " Read the device state before doing anything else.": [
    " 请先读取设备状态，再执行其他操作。",
    " 請先讀取裝置狀態，再執行其他操作。"
  ],
  "The output keeps whatever state it had, which is unknown from here. Reconnect the USB cable.": [
    "输出会保持原来的状态，目前无法确认。请重新连接 USB 数据线。",
    "輸出會保持原來的狀態，目前無法確認。請重新連接 USB 資料線。"
  ],
  "Switching off does not drain it — hold Discharge to bring it to ≤ 1 V.": [
    "关闭不会释放残余电压。长按“主动泄放”，降至 ≤ 1 V。",
    "關閉不會釋放殘餘電壓。長按「主動洩放」，降至 ≤ 1 V。"
  ],
  "Unreadable": [
    "读数无效",
    "讀數無效"
  ],
  "Invalid reading — not zero volts": [
    "读数无效，不代表零伏",
    "讀數無效，不代表零伏"
  ],
  "· to load": [
    "· 负载端",
    "· 負載端"
  ],
  "· source": [
    "· 电源端",
    "· 電源端"
  ],
  "live": [
    "实时",
    "即時"
  ],
  "paused": [
    "已暂停",
    "已暫停"
  ],
  "No live data": [
    "暂无实时数据",
    "暫無即時資料"
  ],
  "Drain": [
    "泄放",
    "洩放"
  ],
  "UID copied": [
    "已复制 UID",
    "已複製 UID"
  ],
  "Copy": [
    "复制",
    "複製"
  ],
  "Could not copy UID": [
    "无法复制 UID",
    "無法複製 UID"
  ],
  "Port": [
    "串口",
    "序列埠"
  ],
  "Firmware": [
    "固件",
    "韌體"
  ],
  "Hardware": [
    "硬件",
    "硬體"
  ],
  "Round trip": [
    "往返延迟",
    "往返延遲"
  ],
  "since": [
    "始于",
    "始於"
  ],
  "at": [
    "发生于",
    "發生於"
  ],
  "Active": [
    "持续中",
    "持續中"
  ],
  "Resolved": [
    "已恢复",
    "已恢復"
  ],
  "Event": [
    "事件",
    "事件"
  ],
  "No faults. Everything is working.": [
    "没有故障记录。",
    "沒有故障紀錄。"
  ],
  "Nothing has happened yet.": [
    "暂无活动记录。",
    "暫無活動紀錄。"
  ],
  "device": [
    "台设备",
    "台裝置"
  ],
  "devices": [
    "台设备",
    "台裝置"
  ],
  "powered": [
    "已开启",
    "已開啟"
  ],
  "off": [
    "已关闭",
    "已關閉"
  ],
  "need attention": [
    "需要关注",
    "需要留意"
  ],
  "Web": [
    "网页",
    "網頁"
  ],
  "Terminal": [
    "终端",
    "終端機"
  ],
  "AI agent": [
    "AI 助手",
    "AI 助手"
  ],
  "API": [
    "API",
    "API"
  ],
  "switched on": [
    "已开启供电",
    "已開啟供電"
  ],
  "switched off": [
    "已关闭供电",
    "已關閉供電"
  ],
  "discharged": [
    "已完成泄放",
    "已完成洩放"
  ],
  "power-cycled": [
    "已完成断电重启",
    "已完成斷電重啟"
  ],
  "renamed": [
    "已修改名称",
    "已修改名稱"
  ],
  "switch on": [
    "开启供电",
    "開啟供電"
  ],
  "switch off": [
    "关闭供电",
    "關閉供電"
  ],
  "discharge": [
    "泄放",
    "洩放"
  ],
  "power-cycle": [
    "断电重启",
    "斷電重啟"
  ],
  "rename": [
    "修改名称",
    "修改名稱"
  ],
  "Discharge timed out": [
    "泄放超时",
    "洩放逾時"
  ],
  "Discharge stopped: output unreadable": [
    "泄放停止：输出电压读数无效",
    "洩放停止：輸出電壓讀數無效"
  ],
  "Internal error": [
    "内部错误",
    "內部錯誤"
  ],
  "Input voltage unreadable": [
    "输入电压读数无效",
    "輸入電壓讀數無效"
  ],
  "Output voltage unreadable": [
    "输出电压读数无效",
    "輸出電壓讀數無效"
  ],
  "Restarted": [
    "设备已重启",
    "裝置已重新啟動"
  ],
  "Request rejected": [
    "请求被拒绝",
    "請求遭拒"
  ],
  "Slow reply": [
    "响应超时",
    "回應逾時"
  ],
  "Noisy connection": [
    "通信校验错误",
    "通訊校驗錯誤"
  ],
  "Late reply discarded": [
    "已丢弃迟到的响应",
    "已捨棄延遲的回應"
  ],
  "Port in use by another app": [
    "串口被其他应用占用",
    "序列埠遭其他應用程式占用"
  ],
  "Disconnected": [
    "连接断开",
    "連線中斷"
  ],
  "Not responding": [
    "设备无响应",
    "裝置無回應"
  ],
  "Identity mismatch": [
    "设备身份不匹配",
    "裝置身分不符"
  ],
  "Connection error": [
    "连接错误",
    "連線錯誤"
  ],
  "Can't write logs": [
    "无法写入日志",
    "無法寫入日誌"
  ],
  "Language": [
    "语言",
    "語言"
  ],
  "Theme": [
    "外观",
    "外觀"
  ],
  "System": [
    "跟随系统",
    "跟隨系統"
  ],
  "Light": [
    "浅色",
    "淺色"
  ],
  "Dark": [
    "深色",
    "深色"
  ],
  "Your power, under control.": [
    "掌控每一次通断。",
    "掌控每一次通斷。"
  ],
  "USB-C SWITCH / LOCAL CONTROL": [
    "USB-C 开关 / 本机控制",
    "USB-C 開關 / 本機控制"
  ],
  "Hold a switch to change power. Open a card for discharge, power cycle and device settings.": [
    "长按开关切换供电，打开卡片可泄放、断电重启和设置设备。",
    "長按開關切換供電，開啟卡片可洩放、斷電重啟和設定裝置。"
  ],
  "OFF ≠ 0 V": [
    "关闭 ≠ 0 V",
    "關閉 ≠ 0 V"
  ],
  "Read back. Then act.": [
    "读回确认，再继续。",
    "讀回確認，再繼續。"
  ],
  "{0}s ago": [
    "{0} 秒前",
    "{0} 秒前"
  ],
  "{0}m ago": [
    "{0} 分钟前",
    "{0} 分鐘前"
  ],
  "{0} {1}": [
    "{0} {1}",
    "{0} {1}"
  ],
  "{0} was already {1}": [
    "{0} 已处于目标状态：{1}",
    "{0} 已處於目標狀態：{1}"
  ],
  "{0}: result unknown": [
    "{0}：结果未知",
    "{0}：結果未知"
  ],
  "Couldn't {0} {1}": [
    "无法为 {1} {0}",
    "無法為 {1} {0}"
  ],
  "{0} is locked": [
    "{0} 已锁定",
    "{0} 已鎖定"
  ],
  "{0}, {1}. Open details": [
    "{0}，{1}。打开详情",
    "{0}，{1}。開啟詳情"
  ],
  "{0} · output keeps its last state": [
    "{0} · 输出保持原状态",
    "{0} · 輸出保持原狀態"
  ],
  "Load still holds {0} V": [
    "负载端仍有 {0} V",
    "負載端仍有 {0} V"
  ],
  "The load still holds {0} V. ": [
    "负载端仍有 {0} V。",
    "負載端仍有 {0} V。"
  ],
  "{0}/24 bytes · too long": [
    "{0}/24 字节 · 名称过长",
    "{0}/24 位元組 · 名稱過長"
  ],
  "Stored on the device · {0}/24 bytes": [
    "保存在设备中 · {0}/24 字节",
    "儲存在裝置中 · {0}/24 位元組"
  ],
  "Sampled {0} · {1}": [
    "采样于{0} · {1}",
    "取樣於{0} · {1}"
  ],
  "{0} · {1} s · on": [
    "{0} · 等待 {1} 秒 · 开启",
    "{0} · 等待 {1} 秒 · 開啟"
  ],
  "{0} · protocol {1}": [
    "{0} · 协议 {1}",
    "{0} · 協定 {1}"
  ],
  "Output voltage, last minute, 0 to {0} V": [
    "最近一分钟输出电压，0 至 {0} V",
    "最近一分鐘輸出電壓，0 至 {0} V"
  ],
  "{0}: {1} — result unknown": [
    "{0}：{1} — 结果未知",
    "{0}：{1} — 結果未知"
  ],
  "by {0}": [
    "来自{0}",
    "來自{0}"
  ],
  "locked": [
    "已锁定",
    "已鎖定"
  ],
  "unlocked": [
    "已解锁",
    "已解鎖"
  ],
  "Discharge stopped after its 5 s limit with output OFF. Check the load for a source keeping VOUT above 1 V before retrying.": [
    "泄放达到 5 秒上限后已停止，输出保持关闭。再次尝试前，请检查负载端是否有其他电源使 VOUT 高于 1 V。",
    "洩放達到 5 秒上限後已停止，輸出保持關閉。再次嘗試前，請檢查負載端是否有其他電源使 VOUT 高於 1 V。"
  ],
  "Discharge stopped because VOUT was invalid. Check the board's ground connection and VOUT sensing; do not assume the load is discharged.": [
    "因 VOUT 读数无效，泄放已停止。请检查电路板接地和 VOUT 采样；不要认定负载已经放电。",
    "因 VOUT 讀數無效，洩放已停止。請檢查電路板接地和 VOUT 取樣；不要認定負載已經放電。"
  ],
  "The device latched an internal error. Read status and diagnostics, then power-cycle the Voloo's USB connection if it persists.": [
    "设备锁存了内部错误。请读取状态和诊断；若问题持续，请断开并重新连接 Voloo 的 USB。",
    "裝置鎖存了內部錯誤。請讀取狀態和診斷；若問題持續，請拔除並重新連接 Voloo 的 USB。"
  ],
  "VIN measurement is invalid; readings are not zero volts. Check SOURCE wiring and the PWR_GND/BUS_GND connection.": [
    "VIN 测量无效，不代表零伏。请检查 SOURCE 接线和 PWR_GND/BUS_GND 连接。",
    "VIN 測量無效，不代表零伏。請檢查 SOURCE 接線和 PWR_GND/BUS_GND 連接。"
  ],
  "VOUT measurement is invalid; readings are not zero volts. Check the load wiring and the PWR_GND/BUS_GND connection.": [
    "VOUT 测量无效，不代表零伏。请检查负载接线和 PWR_GND/BUS_GND 连接。",
    "VOUT 測量無效，不代表零伏。請檢查負載接線和 PWR_GND/BUS_GND 連接。"
  ],
  "The device restarted (outputs reset to OFF). Re-read status before any action; previous operations are not resumed.": [
    "设备已重启，输出已复位为关闭。请先重新读取状态；此前的操作不会自动继续。",
    "裝置已重新啟動，輸出已重設為關閉。請先重新讀取狀態；先前的操作不會自動繼續。"
  ],
  "The device rejected the request. Check firmware version and arguments; do not blindly resend power commands.": [
    "设备拒绝了请求。请检查固件版本和参数，不要盲目重发电源命令。",
    "裝置拒絕了請求。請檢查韌體版本和參數，不要盲目重送電源命令。"
  ],
  "The device is busy with another action. Read status and wait for it to finish.": [
    "设备正在执行其他操作。请读取状态，等待该操作完成。",
    "裝置正在執行其他操作。請讀取狀態，等待該操作完成。"
  ],
  "No reply in time. Read status before any further power action; the command may or may not have executed.": [
    "未及时收到响应。命令可能已经执行，也可能没有执行；进一步操作前，请先读取状态。",
    "未及時收到回應。命令可能已經執行，也可能沒有執行；進一步操作前，請先讀取狀態。"
  ],
  "Corrupted frames were seen. Try another USB cable or port; avoid unpowered hubs.": [
    "检测到损坏的数据帧。请更换 USB 数据线或接口，避免使用无独立供电的集线器。",
    "偵測到損壞的資料框。請更換 USB 資料線或連接埠，避免使用無獨立供電的集線器。"
  ],
  "A reply arrived after its request timed out and was discarded. Read status to confirm the device state.": [
    "超时后收到的响应已被丢弃。请读取状态，确认设备的当前状态。",
    "逾時後收到的回應已被捨棄。請讀取狀態，確認裝置的目前狀態。"
  ],
  "The serial port is in use by another program (for example a browser Web Serial page). Close it and retry.": [
    "串口正被其他程序占用，例如浏览器中的 Web Serial 页面。关闭占用程序后重试。",
    "序列埠正遭其他程式占用，例如瀏覽器中的 Web Serial 頁面。關閉占用程式後重試。"
  ],
  "The USB connection was lost. The output keeps its last state; reconnect and read status before acting.": [
    "USB 连接断开，输出保持原状态。请重新连接并先读取状态，再执行操作。",
    "USB 連線中斷，輸出保持原狀態。請重新連接並先讀取狀態，再執行操作。"
  ],
  "The device stopped answering; its output state is unknown. Replug the USB cable and read status before acting.": [
    "设备不再响应，输出状态未知。请重新插拔 USB 数据线并先读取状态，再执行操作。",
    "裝置不再回應，輸出狀態未知。請重新插拔 USB 資料線並先讀取狀態，再執行操作。"
  ],
  "The UID read from the device does not match the USB serial number. Do not control this port; report it with `voloo report`.": [
    "设备返回的 UID 与 USB 序列号不一致。请勿控制该串口，使用 `voloo report` 生成报告。",
    "裝置傳回的 UID 與 USB 序號不一致。請勿控制該序列埠，使用 `voloo report` 產生報告。"
  ],
  "Serial I/O failed. Reconnect the device and read status before acting.": [
    "串口读写失败。请重新连接设备并先读取状态，再执行操作。",
    "序列埠讀寫失敗。請重新連接裝置並先讀取狀態，再執行操作。"
  ],
  "Logs could not be written. Check free space and permissions of the Voloo home directory.": [
    "无法写入日志。请检查 Voloo 数据目录的剩余空间和权限。",
    "無法寫入日誌。請檢查 Voloo 資料目錄的剩餘空間和權限。"
  ]
};
function matchLanguage(value) {
  const lang = String(value || '').toLowerCase();
  if (lang.startsWith('zh')) return /hant|tw|hk|mo/.test(lang) ? 'zh-Hant' : 'zh-Hans';
  return 'en';
}
function readPreference(key) { try { return localStorage.getItem(key); } catch { return null; } }
function savePreference(key, value) { try { localStorage.setItem(key, value); } catch { /* private or restricted storage */ } }
const saved = readPreference('voloo.language');
let language = ['en', 'zh-Hans', 'zh-Hant'].includes(saved) ? saved : matchLanguage(navigator.languages?.[0] || navigator.language);
function tr(input, ...values) {
  if (input == null) return input;
  const key = Array.isArray(input) ? input.reduce((s, part, i) => s + (i ? `{${i - 1}}` : '') + part, '') : String(input);
  const text = language === 'en' ? key : messages[key]?.[language === 'zh-Hans' ? 0 : 1] ?? key;
  return text.replace(/\{(\d+)\}/g, (all, i) => values[i] == null ? all : String(values[i]));
}
function localized(object) { return new Proxy(object, { get: (target, key) => tr(target[key]) }); }
globalThis.VolooI18n = { tr, localized, messages, matchLanguage, readPreference, savePreference,
  get language() { return language; },
  setLanguage(value) { language = matchLanguage(value); savePreference('voloo.language', language); document.documentElement.lang = language; },
};
document.documentElement.lang = language;
})();
