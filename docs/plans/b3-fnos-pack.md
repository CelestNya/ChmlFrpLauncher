# B3 实施计划：fnos-pack（.fpk 打包）✅ 已完成并验收

> 状态：2026-08-11 全部交付。结构级验收通过；真机安装/网关实测留待 fnOS 设备（见"遗留"）。

## 形态与关键事实（调研结论，来自 fnOS 官方开发指南）
- **.fpk = tar.gz**：`app.tgz`（target 目录）+ `cmd/` + `config/` + `ui/` + `manifest`（INI 格式）+ 图标 + `*.sc` 端口转发
- **manifest 是 INI 非 JSON**：`appname/version/display_name/desc/source` 必填；`service_port` 单端口；`platform` 只支持单值（x86/arm）→ 双架构 = 出两个 .fpk
- **cmd/main**：`start`（拉起 daemon）`stop` `status`（运行 0 / 未运行 3）`log`；TRIM_APPDEST / TRIM_PKGVAR / TRIM_SERVICE_PORT / TRIM_TEMP_LOGFILE 环境变量注入
- **ui/config**：桌面入口 JSON，`type: "iframe"` 即桌面窗口内嵌加载（ChmlFrp 无原生 GUI 的形态）
- **config/privilege**：`run-as: "package"`（以专用低权限用户运行）
- **config/resource**：`port-config.protocol-file` 指向 `*.sc`；systemd-unit 留空（daemon 自行管理）
- 参考实现：conversun/fnos-apps（mihomo 为 daemon+iframe 同形态范例）

## 交付物
| 文件 | 说明 |
|------|------|
| `fnos-pack/fpk/manifest` | 应用标识（appname=chmlfrp，v0.7.5，platform 构建期改写，service_port=17890） |
| `fnos-pack/fpk/config/privilege` | run-as=package + chmlfrp 用户 |
| `fnos-pack/fpk/config/resource` | 端口转发协议文件声明 |
| `fnos-pack/fpk/ChmlFrp.sc` | 17890/tcp 端口转发 |
| `fnos-pack/fpk/ui/config` | iframe 桌面入口（chmlfrp.Application） |
| `fnos-pack/fpk/cmd/main` | 生命周期：start/stop/status/log（管理 daemon，frpc 由 daemon 内守护） |
| `fnos-pack/build-fpk.sh` | 全链打包：前端（apply-patches.sh）→ daemon release 编译 → 图标缩放 → app.tgz → .fpk；`--arch` 双架构、`--skip-frontend` 复用产物 |

## 验收结果（WSL 已执行）
1. ✅ `build-fpk.sh` 全链 exit 0，产出 `chmlfrp_0.7.5_x86.fpk`
2. ✅ .fpk 内部结构完整：app.tgz / cmd/main / config/privilege / config/resource / ui/config / ui/images / ICON.PNG / ICON_256.PNG / ChmlFrp.sc / manifest
3. ✅ app.tgz 内容：`chmlfrp-daemon`（release 二进制）+ `dist/`（shim 注入后的前端）
4. ✅ manifest `checksum` 与 app.tgz 实际 md5 一致
5. ✅ cmd/main 可执行位正确（-rwxr-xr-x）
6. ✅ cmd/main 生命周期冒烟：start → status(0) → daemon 监听 17891 → log → stop(0) → status(3) → pid 清除
   （修复：stop 分支 `local` 在顶层非法，改用普通变量）

## 遗留（需 fnOS 真机 / CI 验证）
- 应用中心安装/启动/停止/升级/卸载全流程
- 统一网关 iframe 加载（gatewayPrefix/gatewaySocket、SPA fallback、公网域名访问）
- fnOS 是否随系统启动应用（隧道自启链路）
- OAuth 设备码 CORS（iframe 环境）
- arm64 .fpk 交叉编译（需 rustup target + 交叉工具链，脚本已支持）

## 风险提示
- `service_port=17890` 与桌面默认端口一致；若与 fnOS 系统端口冲突，改 manifest 一处即可
- build-fpk.sh 依赖 PIL 生成图标；无 PIL 时跳过图标（.fpk 缺 ICON 不可装，CI 需装 Pillow）
- daemon 数据目录 TRIM_PKGVAR 由 fnOS 注入；本地模拟用 DAEMON_DATA_DIR/TRIM_PKGVAR
