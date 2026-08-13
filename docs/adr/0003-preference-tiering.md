# ADR-0003：偏好分层——渲染类留守 localStorage

**状态**: accepted
**决定**: Preference 分两层。**渲染类**（theme、themeFollowSystem、effectType、
backgroundImage、backgroundBlur、backgroundOverlayOpacity、background_playlist
索引等）**留守 localStorage**；**行为类**（NodeSetting，见 ADR-0002）进后端。
`showTitleBar`/`closeBehavior` 标记 desktop-only 不迁移。

**为什么**: 渲染类偏好是首帧**同步惰性初始化**（`useBackground.ts:33-36`、
`useAppTheme.ts:31` 用 `useState(() => localStorage.getItem(...))`），首帧渲染
直接依赖。移后端异步拉取会导致首帧空背景 / 主题闪烁（flash of wrong theme）——
这正是桌面版当初用同步读避免的问题。留守 localStorage 是「首帧时序」约束下的
唯一无副作用选项。

**被否决**: 全部偏好后端化（曾考虑 daemon 注入 boot blob / tauri-plugin-store
同步缓存副本）——工程量大、要处理 FOUC、远超本次重构范围；且用户已确认接受
「渲染类留守，跨设备一致仅对行为类」。

**代价**: 渲染类偏好仍跟浏览器走（清缓存丢失），不跨设备同步——这是接受
「首帧体验 > 跨设备一致」的明确权衡。
