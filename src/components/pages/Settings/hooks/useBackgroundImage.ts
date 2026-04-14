import { useState, useEffect, useRef } from "react";
import { toast } from "sonner";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import {
  getInitialBackgroundImage,
  getInitialBackgroundOverlayOpacity,
  getInitialBackgroundBlur, isVideoFile,
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

  // 幻灯片状态持久化初始化
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

  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // 基础配置持久化
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

  // 幻灯片配置持久化
  useEffect(() => {
    localStorage.setItem("background_playlist", JSON.stringify(playlist));
  }, [playlist]);

  useEffect(() => {
    localStorage.setItem("background_current_index", currentIndex.toString());
  }, [currentIndex]);

  useEffect(() => {
    localStorage.setItem("background_interval_time", intervalTime.toString());
  }, [intervalTime]);

// 核心计时器逻辑
  useEffect(() => {
    // 每次 intervalTime 或 playlist 变化，先彻底清理旧计时器
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }

    // 只有在有时间且列表有图时才开启
    if (intervalTime > 0 && playlist.length > 0) {
      timerRef.current = setInterval(() => {
        setCurrentIndex((prev) => {
          const next = (prev + 1) % playlist.length;
          const nextImage = playlist[next];

          // 关键：在这里直接更新 backgroundImage，确保 App.tsx 拿到新图
          if (nextImage) {
            setBackgroundImage(nextImage);
          }
          return next;
        });
      }, intervalTime);
    }

    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, [intervalTime, playlist]); // 不要把 currentIndex 加进来，否则计时器每秒都会重启

  // 选择文件夹并读取图片
  const handleSelectFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
      });

      if (selected) {
        const files = await invoke<string[]>("read_image_folder", {
          dirPath: typeof selected === 'string' ? selected : selected[0]
        });

        if (files.length > 0) {
          const formattedFiles = files.map(path => `app://${path}`);
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
        filters: [{ name: "媒体文件", extensions: [...IMAGE_EXTENSIONS, "mp4", "webm"] }],
      });

      const filePath = Array.isArray(selected) ? selected[0] : selected;

      if (filePath) {
        // 选单张时，先彻底重置幻灯片状态
        setPlaylist([]);
        setIntervalTime(0);
        setCurrentIndex(0);

        const isVideo = isVideoFile(filePath);
        const command = isVideo ? "copy_background_video" : "copy_background_image";

        const copiedPath = await invoke<string>(command, { sourcePath: filePath });
        const finalPath = `app://${copiedPath}`;

        // 更新背景：如果新路径和旧路径一样，可以考虑先置 null 再设置
        // 但通常直接设置即可触发 BackgroundLayer 的渲染
        setBackgroundImage(finalPath);
        toast.success("背景设置成功");
      }
    } catch (error) {
      console.error("Selection error:", error);
      toast.error(`选择文件失败`);
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
    currentIndex
  };
}