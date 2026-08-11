# B3 实施计划：fnos-pack（.fpk 打包）✅ 已完成并验收（含真机被拒修正）

> 状态：2026-08-11 交付。**真机安装首版被拒**（应用包不符合系统要求）→ 对照官方 fnpack 重构后通过校验。
> 本批核心教训：**必须用官方 fnpack 工具打包**（手写 tar 平铺结构不符合 fnOS 包规范）。

## ⚠️ 真机被拒根因（2026-08-11 实测，重要）
首版手写 tar 的 .fpk 结构 = `app.tgz + cmd/ + config/ + ui/(根级) + manifest + ICON + ChmlFrp.sc`，安装报「应用包不符合系统要求」。
对照**官方 fnpack 1.2.3 生成的骨架**与官方 fnnas/appstore.* 仓库，差异如下：
1. **目录布局**：官方是 fnpack 项目 = `app/`（含 `ui/`）+ `cmd/` + `config/` + `wizard/` + `manifest` + `ICON.PNG` + `ICON_256.PNG`；`desktop_uidir` 相对于 `app/`（`app/ui/`），**不是根级 ui/**
2. **app.tgz 生成**：fnpack 把 `app/` 内容**平铺**压成 app.tgz → 安装后 TRIM_APPDEST 下直接是 `chmlfrp-daemon` + `dist/` + `ui/`（无 app/ 层）
3. **缺 wizard/ 目录**（骨架必须存在，可空 .gitkeep）
4. **cmd/ 缺生命周期脚本集**（骨架有 8 个：install/upgrade/uninstall/config 的 init+callback）
5. **manifest 无 checksum**：fnpack 自动补 `checksum`（app.tgz md5）
6. **config/resource**：官方模板用 `data-share`（非 port-config），ChmlFrp.sc 端口转发非必需（iframe 入口即可）
7. **privilege**：官方模板不带 username/groupname（run-as=package 即可）

## 官方 fnpack 契约（修正后依据）
- 工具下载：`https://static2.fnnas.com/fnpack/fnpack-1.2.3-{linux-amd64|linux-arm64|darwin-*}`；`fnpack create` 生成骨架、`fnpack build` 出包并预检
- 预检项：manifest 必填字段 / config JSON 合法性 / ICON.PNG+ICON_256.PNG / app/ / cmd/ / wizard/ / `app/{desktop_uidir}/`
- manifest 格式：`key = value`（对齐列宽），`desktop_applaunchname` 对应 app/ui/config 的 entry ID
- app.tgz = `app/` 内容平铺（daemon 二进制放 app/ 根）

## 交付物（修正后）
| 文件 | 说明 |
|------|------|
| `fnos-pack/fpk/manifest` | appname=chmlfrp、v0.7.5、platform 构建期改写、service_port=17890、ctl_stop |
| `fnos-pack/fpk/app/chmlfrp-daemon` | 构建期放入 daemon 二进制（app/ 根） |
| `fnos-pack/fpk/app/dist/` | 构建期放入前端产物（shim 注入后） |
| `fnos-pack/fpk/app/ui/config` | iframe 桌面入口（chmlfrp.Application） |
| `fnos-pack/fpk/cmd/main` | 生命周期 start/stop/status/log（daemon 常驻，frpc 内守护） |
| `fnos-pack/fpk/cmd/{install,upgrade,uninstall,config}_{init,callback}` | 生命周期钩子（无额外操作） |
| `fnos-pack/fpk/config/{privilege,resource}` | run-as=package + data-share |
| `fnos-pack/fpk/wizard/.gitkeep` | 向导目录（空） |
| `fnos-pack/build-fpk.sh` | 全链：前端→daemon→组装 fnpack 项目→fnpack build；`--arch` 双架构、`--bundle` 自更新包、自动下载 fnpack（含重试） |

## 验收结果（WSL 已执行，修正后）
1. ✅ 官方 fnpack build 通过（Verifying files → Packing successfully），产物 `chmlfrp_0.7.5_x86.fpk`
2. ✅ .fpk 结构 = app.tgz + cmd/(9 脚本) + config/ + wizard/ + manifest + ICON*.PNG
3. ✅ app.tgz = TRIM_APPDEST 内容：chmlfrp-daemon + dist/ + ui/（平铺）
4. ✅ manifest 自动含 checksum
5. ✅ cmd/main 生命周期冒烟：start → status(0) → bootstrap 0.7.5 → stop → status(3) → pid 清除
6. ✅ build-fpk.sh --bundle 正常

## 遗留（需 fnOS 真机验证）
- 应用中心安装/启动/停止/升级/卸载全流程（本次修正后需重新装）
- 统一网关 iframe 加载（gatewayPrefix/gatewaySocket、SPA fallback、公网域名访问）
- fnOS 是否随系统启动应用（隧道自启链路）
- OAuth 设备码 CORS（iframe 环境）
- arm64 .fpk 交叉编译 + 真机
- target 目录写权限（run-as=package）→ 自更新 apply 落点
