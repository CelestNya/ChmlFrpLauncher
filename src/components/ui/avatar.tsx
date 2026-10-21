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
 * 取回经 Rust 侧高质量缩放后的头像 dataURL。
 *
 * 背景：头像原图通常 640x640，界面只显示 40~44px，需 14~16 倍降采样。
 * WebKitGTK 的 <img> 在大比例降采样时滤波质量不足，细线条会被稀疏采样成
 * 暗色噪点；Chromium 实现更好，所以 Windows 上不明显。实测暗像素比例约
 * 1.9%，经 Rust 侧 Lanczos3 逐级减半后降到约 1.2%，且 CSS image-rendering
 * 无法改善该降采样路径。
 *
 * 失败时回退原始 URL，保证头像至少能显示。
 */
const avatarCache = new Map<string, string>();

function useSharpenedAvatar(src: string | undefined): string | undefined {
  const [out, setOut] = React.useState<string | undefined>(src);

  React.useEffect(() => {
    if (!src) {
      setOut(undefined);
      return;
    }
    if (!src.startsWith("http")) {
      setOut(src);
      return;
    }
    const cached = avatarCache.get(src);
    if (cached) {
      setOut(cached);
      return;
    }

    let cancelled = false;
    setOut(src);

    invoke<{ data_url: string; width: number; height: number }>(
      "fetch_avatar_sharp",
      { url: src },
    )
      .then((res) => {
        if (cancelled) return;
        if (res && res.data_url) {
          avatarCache.set(src, res.data_url);
          setOut(res.data_url);
        }
      })
      .catch(() => {
        /* 静默回退到原图 */
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
  const sharpened = useSharpenedAvatar(
    typeof src === "string" ? src : undefined,
  );

  return (
    <AvatarPrimitive.Image
      data-slot="avatar-image"
      className={cn("aspect-square size-full", className)}
      src={sharpened}
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
