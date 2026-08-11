# B3 实施计划：fnos-pack（.fpk 打包）✅ 已完成并验收（含真机两次修复）

> 状态：2026-08-11 交付。**真机实测**：首次被拒（包结构）→ 官方 fnpack 重构后装上但白屏（GLIBC 不兼容）→ musl 静态编译后**真机运行成功**。
> 本批核心教训：① 必须用官方 fnpack 工具打包；② **daemon 必须 musl 静态编译**（fnOS 基底 Debian 12 glibc 2.36）。

## ⚠️ 真机修复记录（2026-08-11 实测）
### 第一轮：安装被拒「应用包不符合系统要求」
对照官方 fnpack 1.2.3 骨架 + fnnas/appstore.* 仓库，修复：
1. 目录布局 → `app/`（含 `ui/`）+ `cmd/` + `config/` + `wizard/` + manifest + 图标；`desktop_uidir` 相对 `app/`
2. app.tgz = `app/` 内容平铺（TRIM_APPDEST 下直接是 chmlfrp-daemon + dist/ + ui/）
3. 补 wizard/ 目录、cmd/ 生命周期脚本集（8 个）
4. manifest 由 fnpack 自动补 checksum
5. config/resource 用官方 `data-share` 模板
→ fnpack build 预检+出包通过，真机安装成功。

### 第二轮：装上但 iframe 白屏/闪退（关键）
现象：首次打开白屏后闪退，二次空白不闪退。
**根因（真机 error.log + 实测）**：daemon 在 WSL Ubuntu 26.04（glibc 2.43）上编译，二进制要求 `GLIBC_2.39`；fnOS = Debian 12 glibc 2.36 → 加载即崩：
```
/vol1/@appcenter/chmlfrp/chmlfrp-daemon: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found
```
**修复**：daemon **musl 静态编译**（`x86_64-unknown-linux-musl`，rustls 无系统库依赖）→ `statically linked`、零 GLIBC 依赖 → 任何 Linux 可直接运行。
- build-fpk.sh 编译步骤改为 musl 静态，并校验 `file` 输出含 `statically linked` 否则拒绝打包
- CI workflow 依赖加 `musl-tools`
- 真机验证：安装成功 → daemon 存活 → 监听 17890 → bootstrap/SPA/shim/invoke 全通

## 官方 fnpack 契约（修正后依据）
- 工具下载：`https://static2.fnnas.com/fnpack/fnpack-1.2.3-{linux-amd64|linux-arm64|darwin-*}`；`fnpack create` 生成骨架、`fnpack build` 出包并预检
- 预检项：manifest 必填字段 / config JSON / ICON.PNG+ICON_256.PNG / app/ / cmd/ / wizard/ / `app/{desktop_uidir}/`
- manifest：`key = value` 对齐列宽；`desktop_applaunchname` ↔ app/ui/config entry ID
- **运行时兼容**：任何原生二进制必须静态链接或匹配 fnOS glibc（Debian 12 = glibc 2.36），NAS 分发标准做法 = musl 静态

## 交付物（修正后）
| 文件 | 说明 |
|------|------|
| `fnos-pack/fpk/manifest` | appname=chmlfrp、v0.7.5、platform 构建期改写、service_port=17890、ctl_stop |
| `fnos-pack/fpk/app/chmlfrp-daemon` | 构建期放入 daemon（musl 静态） |
| `fnos-pack/fpk/app/dist/` | 前端产物（shim 注入后） |
| `fnos-pack/fpk/app/ui/config` | iframe 桌面入口（chmlfrp.Application） |
| `fnos-pack/fpk/cmd/{main,install_*,upgrade_*,uninstall_*,config_*}` | 生命周期脚本集 |
| `fnos-pack/fpk/config/{privilege,resource}` | run-as=package + data-share |
| `fnos-pack/fpk/wizard/.gitkeep` | 向导目录（空） |
| `fnos-pack/build-fpk.sh` | 全链：前端→daemon musl 静态→fnpack build；`--arch`/`--bundle`/自动下载 fnpack |

## 验收结果（真机 + WSL，修正后）
1. ✅ 真机安装成功（appcenter-cli install-fpk -v 1）
2. ✅ daemon 进程存活、监听 127.0.0.1:17890
3. ✅ bootstrap 返回 0.7.5；SPA 首页 + shim 注入正常
4. ✅ invoke 透传（get_frpc_directory / check_frpc_exists / 未知命令错误）
5. ✅ build-fpk.sh 编译 musl 静态 + `file` 校验通过

## 遗留（仍需验证）
- iframe 桌面入口打开后的完整 UI 交互（登录→隧道→日志）——daemon 侧已通，浏览器侧待用户确认
- arm64 交叉（musl 需 aarch64-linux-musl-gcc 工具链，CI 未配置）
- OAuth 设备码 CORS（iframe 环境）
- 统一网关细节（gatewayPrefix/gatewaySocket、公网域名访问）
