import * as React from "react";
import * as AvatarPrimitive from "@radix-ui/react-avatar";
import { invoke } from "@tauri-apps/api/core";

import { cn } from "../../lib/utils";

function Avatar({
  className,
  ...props
}: React.ComponentProps<typeof AvatarPrimitive.Root>) {
  return (
    <AvatarPrimitive.Root
      data-slot="avatar"
      className={cn(
        "relative flex size-8 shrink-0 overflow-hidden rounded-full",
        className,
      )}
      {...props}
    />
  );
}

/**
 * 把头像 URL 的尺寸参数改小后再交给 <img>。
 *
 * 背景：头像原图通常 640x640，界面只显示 40~44px，需要 15 倍以上降采样。
 * WebKitGTK 的 <img> 在大比例降采样时滤波质量不足，细线条会被稀疏采样成暗色
 * 噪点；Chromium 的实现更好，所以同样代码在 Windows 上噪点不明显。该问题不受
 * CSS image-rendering 影响。
 *
 * 处理方式刻意保持最小：只把 URL 的 spec 参数从 640 改为 100（服务端支持按需
 * 缩放），图片仍由服务端缩放成 JPEG、由浏览器原样加载，因此格式不变、浏览器
 * HTTP 缓存照常生效（上游响应带 Cache-Control: max-age=30 天），本地不做任何
 * 像素处理。
 *
 * 与上游的唯一差别就是这一个查询参数；若 URL 不含 spec 或解析失败，原样返回。
 */
function useSmallerAvatarUrl(src: string | undefined): string | undefined {
  const [out, setOut] = React.useState<string | undefined>(src);

  React.useEffect(() => {
    if (!src || !src.startsWith("http")) {
      setOut(src);
      return;
    }

    let cancelled = false;
    invoke<{ url: string }>("resolve_avatar_url", { url: src })
      .then((res) => {
        if (!cancelled && res && res.url) setOut(res.url);
      })
      .catch(() => {
        if (!cancelled) setOut(src);
      });

    return () => {
      cancelled = true;
    };
  }, [src]);

  return out;
}

function AvatarImage({
  className,
  src,
  ...props
}: React.ComponentProps<typeof AvatarPrimitive.Image>) {
  const resolved = useSmallerAvatarUrl(
    typeof src === "string" ? src : undefined,
  );

  return (
    <AvatarPrimitive.Image
      data-slot="avatar-image"
      className={cn("aspect-square size-full", className)}
      src={resolved}
      {...props}
    />
  );
}

function AvatarFallback({
  className,
  ...props
}: React.ComponentProps<typeof AvatarPrimitive.Fallback>) {
  return (
    <AvatarPrimitive.Fallback
      data-slot="avatar-fallback"
      className={cn(
        "bg-muted flex size-full items-center justify-center rounded-full",
        className,
      )}
      {...props}
    />
  );
}

export { Avatar, AvatarImage, AvatarFallback };
