import { useState, useEffect } from "react";
import { toast } from "sonner";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import {
  getInitialBackgroundImage,
  getInitialBackgroundOverlayOpacity,
  getInitialBackgroundBlur,
  isVideoFile,
} from "../utils";

const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

export function useBackgroundImage() {
  const [backgroundImage, setBackgroundImage] = useState<string | null>(() =>
    getInitialBackgroundImage(),
  );
  const [isSelectingImage, setIsSelectingImage] = useState(false);
  const [overlayOpacity, setOverlayOpacity] = useState<number>(() =>
    getInitialBackgroundOverlayOpacity(),
  );
  const [blur, setBlur] = useState<number>(() => getInitialBackgroundBlur());
  const [playlist, setPlaylist] = useState<string[]>(() => {
    const saved = localStorage.getItem("background_playlist");
    return saved ? JSON.parse(saved) : [];
  });
  const [currentIndex, setCurrentIndex] = useState(() => {
    const saved = localStorage.getItem("background_current_index");
    return saved ? parseInt(saved, 10) : 0;
  });
  const [intervalTime, setIntervalTime] = useState<number>(() => {
    const saved = localStorage.getItem("background_interval_time");
    return saved ? parseInt(saved, 10) : 0;
  });

  useEffect(() => {
    localStorage.setItem("backgroundImage", backgroundImage || "");
    window.dispatchEvent(new Event("backgroundImageChanged"));
  }, [backgroundImage]);

  useEffect(() => {
    localStorage.setItem("backgroundOverlayOpacity", overlayOpacity.toString());
    window.dispatchEvent(new Event("backgroundOverlayChanged"));
  }, [overlayOpacity]);

  useEffect(() => {
    localStorage.setItem("backgroundBlur", blur.toString());
    window.dispatchEvent(new Event("backgroundOverlayChanged"));
  }, [blur]);

  useEffect(() => {
    localStorage.setItem("background_playlist", JSON.stringify(playlist));
    window.dispatchEvent(new Event("backgroundSlideshowChanged"));
  }, [playlist]);

  useEffect(() => {
    localStorage.setItem("background_current_index", currentIndex.toString());
    window.dispatchEvent(new Event("backgroundSlideshowChanged"));
  }, [currentIndex]);

  useEffect(() => {
    localStorage.setItem("background_interval_time", intervalTime.toString());
    window.dispatchEvent(new Event("backgroundSlideshowChanged"));
  }, [intervalTime]);

  const handleSelectFolder = async () => {
    // fnOS（ADR-0004）：砍轮播——文件夹导入（webkitdirectory 多图 base64 塞
    // localStorage）是配额压力源。fnOS 只支持单图（handleSelectBackgroundImage）。
    const isFnOS =
      typeof window !== "undefined" &&
      Boolean((window as unknown as { __FNOS__?: boolean }).__FNOS__);
    if (isFnOS) {
      toast.info("fnOS 版暂不支持文件夹轮播，请选择单张图片");
      return;
    }

    try {
      const selected = await open({
        directory: true,
        multiple: false,
      });

      if (selected) {
        const files = await invoke<string[]>("import_background_image_folder", {
          dirPath: typeof selected === "string" ? selected : selected[0],
        });

        if (files.length > 0) {
          const formattedFiles = files.map((path) => `app://${path}`);
          setPlaylist(formattedFiles);
          setCurrentIndex(0);
          setBackgroundImage(formattedFiles[0]);
          setIntervalTime(5000);
          toast.success(`成功导入 ${files.length} 张图片并开启轮播`);
        } else {
          toast.error("文件夹内没有发现可用图片");
        }
      }
    } catch (err) {
      console.error(err);
      toast.error("读取文件夹失败");
    }
  };

  const handleSelectBackgroundImage = async () => {
    if (isSelectingImage) return;
    setIsSelectingImage(true);

    try {
      const selected = await open({
        multiple: false,
        filters: [
          {
            name: "媒体文件",
            extensions: [...IMAGE_EXTENSIONS, "mp4", "webm"],
          },
        ],
      });

      const filePath = Array.isArray(selected) ? selected[0] : selected;

      if (filePath) {
        setPlaylist([]);
        setIntervalTime(0);
        setCurrentIndex(0);

        // fnOS（ADR-0004）：shim 的 dialog.open 已把选图上传 daemon
        // （save_background_image），返回 /assets/backgrounds/<file> 托管 URL；
        // getBackgroundType 识别非 data:/app:// 前缀直接当 src 渲染。
        const isFnOS =
          typeof window !== "undefined" &&
          Boolean(
            (window as unknown as { __FNOS__?: boolean }).__FNOS__,
          );
        if (isFnOS) {
          setBackgroundImage(filePath);
          toast.success("背景设置成功");
          return;
        }

        const isVideo = isVideoFile(filePath);
        const command = isVideo
          ? "copy_background_video"
          : "copy_background_image";

        const copiedPath = await invoke<string>(command, {
          sourcePath: filePath,
        });
        const finalPath = `app://${copiedPath}`;

        setBackgroundImage(finalPath);
        toast.success("背景设置成功");
      }
    } catch (error) {
      console.error("Selection error:", error);
      toast.error("选择文件失败");
    } finally {
      setIsSelectingImage(false);
    }
  };

  const handleClearBackgroundImage = () => {
    setBackgroundImage(null);
    setPlaylist([]);
    setIntervalTime(0);
    setCurrentIndex(0);
    toast.success("已清除背景");
  };

  return {
    backgroundImage,
    isSelectingImage,
    overlayOpacity,
    setOverlayOpacity,
    blur,
    setBlur,
    intervalTime,
    setIntervalTime,
    handleSelectFolder,
    handleSelectBackgroundImage,
    handleClearBackgroundImage,
    playlist,
    currentIndex,
  };
}
