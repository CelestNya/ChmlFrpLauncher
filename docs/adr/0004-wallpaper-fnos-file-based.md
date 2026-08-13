# ADR-0004：壁纸 fnOS 专属砍轮播 + 文件化

**状态**: accepted
**决定**: 壁纸轮播（选文件夹 → 20 张图 base64 → localStorage）**仅 fnOS 砍掉**
（桌面保留）；daemon 恢复 background 文件命令（从桌面
`src-tauri/src/commands/background.rs` 移植），fnOS 前端选图后存**路径**而非
base64，经 daemon 托管 URL + 浏览器缓存渲染。

**为什么**:
1. **资源瓶颈真相**（探索证据）: 壁纸 100% 是用户本地文件，**零网络/服务器依赖**。
   「吃资源」实为 fnOS 下 20 张图 × base64 逼近 localStorage 配额 + 每次渲染把
   数 MB dataURL 塞 DOM——是**浏览器**资源，不是服务器。
2. daemon 之前把 background 命令全部 NO_OP（`invoke.rs:392-395`），fnOS 前端
   退回 base64 是临时降级（`fnos-shim/README.md:32-33` 明确标注 B5 待补）。
3. 轮播砍在 fnOS 侧：桌面是本地文件轮播（不吃资源），是桌面用户可能喜欢的
   功能，不删；fnOS 砍掉是修复 base64 配额，不是功能裁剪。
4. 文件化后 `backgroundImage`/`background_playlist` 存路径（`/vol1/@appdata/
   chmlfrp/backgrounds/xxx.jpg`），顺带解决 M3（localStorage 配额）——数据形态
   变路径后配额问题消失。

**渲染**: fnOS 前端沿用桌面 blobURL 渲染路径（fs 读二进制 → URL.createObjectURL），
source 从 `app://` 换成 daemon 托管 URL。
