import { useMemo, useEffect, useState, useRef, useReducer } from "react";
import type { BackgroundType } from "@/components/App/hooks/useBackground";

interface BackgroundLayerProps {
  backgroundImage: string | null;
  imageSrc: string | null;
  backgroundType: BackgroundType;
  videoSrc: string | null;
  videoLoadError: boolean;
  videoRef: React.RefObject<HTMLVideoElement | null>;
  videoStartSound: boolean;
  overlayOpacity: number;
  blur: number;
  getBackgroundColorWithOpacity: (opacity: number) => string;
  appContainerRef: React.RefObject<HTMLDivElement | null>;
  onVideoError: () => void;
  onVideoLoadedData: () => void;
}

interface TransitionState {
  currentSrc: string | null;
  nextSrc: string | null;
  showNext: boolean;
}

type TransitionAction =
    | { type: "SET_CURRENT"; src: string | null }
    | { type: "START_TRANSITION"; nextSrc: string | null }
    | { type: "SHOW_NEXT" }
    | { type: "FINISH_TRANSITION" };

const transitionReducer = (
    state: TransitionState,
    action: TransitionAction
): TransitionState => {
  switch (action.type) {
    case "SET_CURRENT":
      return {
        currentSrc: action.src,
        nextSrc: null,
        showNext: false,
      };

    case "START_TRANSITION":
      return {
        ...state,
        nextSrc: action.nextSrc,
        showNext: false, // 新图先隐藏
      };

    case "SHOW_NEXT":
      return {
        ...state,
        showNext: true, // 触发 opacity 从 0 -> 1
      };

    case "FINISH_TRANSITION":
      return {
        currentSrc: state.nextSrc,
        nextSrc: null,
        showNext: false,
      };

    default:
      return state;
  }
};

export function BackgroundLayer({
                                  backgroundImage,
                                  imageSrc,
                                  backgroundType,
                                  videoSrc,
                                  videoLoadError,
                                  videoRef,
                                  videoStartSound,
                                  overlayOpacity,
                                  blur,
                                  getBackgroundColorWithOpacity,
                                  appContainerRef,
                                  onVideoError,
                                  onVideoLoadedData,
                                }: BackgroundLayerProps) {
  const [transitionState, dispatch] = useReducer(transitionReducer, {
    currentSrc: imageSrc,
    nextSrc: null,
    showNext: false,
  });

  const { currentSrc, nextSrc, showNext } = transitionState;

  const prevImageSrcRef = useRef<string | null>(imageSrc);

  const [, setIsDark] = useState(() => {
    if (typeof window === "undefined") return false;
    return document.documentElement.classList.contains("dark");
  });

  useEffect(() => {
    const updateTheme = () => {
      setIsDark(document.documentElement.classList.contains("dark"));
      window.dispatchEvent(new Event("themeChanged"));
    };

    const observer = new MutationObserver(updateTheme);
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["class"],
    });

    return () => observer.disconnect();
  }, []);

// ⭐ Crossfade 切换逻辑
  useEffect(() => {
    if (backgroundType !== "image" || !imageSrc) return;

    // 如果是初次加载（即之前 currentSrc 是空的），直接显示，不跑动画
    // 这样可以解决点击单张图或启动时不显示的问题
    if (!currentSrc && imageSrc) {
      dispatch({ type: "SET_CURRENT", src: imageSrc });
      prevImageSrcRef.current = imageSrc;
      return;
    }

    // 如果图片没变，跳过
    if (imageSrc === prevImageSrcRef.current) return;

    // 正常的切换动画逻辑
    dispatch({ type: "START_TRANSITION", nextSrc: imageSrc });
    prevImageSrcRef.current = imageSrc;

    requestAnimationFrame(() => {
      requestAnimationFrame(() => {
        dispatch({ type: "SHOW_NEXT" });
      });
    });

    const timer = setTimeout(() => {
      dispatch({ type: "FINISH_TRANSITION" });
    }, 1650);

    return () => clearTimeout(timer);
  }, [imageSrc, backgroundType,currentSrc]); // 加上 state.currentSrc 依赖

  const overlayStyle = useMemo(() => {
    if (!backgroundImage) return {};
    return {
      backgroundColor: getBackgroundColorWithOpacity(overlayOpacity),
    };
  }, [backgroundImage, overlayOpacity, getBackgroundColorWithOpacity]);

  useEffect(() => {
    const updateColors = () => {
      const overlay = document.querySelector(".background-overlay") as HTMLElement;
      if (overlay) {
        overlay.style.backgroundColor = getBackgroundColorWithOpacity(overlayOpacity);
      }
      if (!backgroundImage && appContainerRef.current) {
        appContainerRef.current.style.backgroundColor =
            getBackgroundColorWithOpacity(100);
      }
    };

    updateColors();
    window.addEventListener("themeChanged", updateColors);
    return () => window.removeEventListener("themeChanged", updateColors);
  }, [backgroundImage, overlayOpacity, getBackgroundColorWithOpacity, appContainerRef]);

  const backgroundBlurStyle = useMemo(() => {
    if (!backgroundImage || blur === 0) return {};
    return {
      filter: `blur(${blur}px)`,
      WebkitFilter: `blur(${blur}px)`,
    };
  }, [backgroundImage, blur]);

  const commonImageStyle: React.CSSProperties = {
    backgroundSize: "cover",
    backgroundPosition: "center",
    backgroundRepeat: "no-repeat",
    ...backgroundBlurStyle,
    zIndex: 0,
    pointerEvents: "none",
  };

  return (
      <div className="absolute inset-0" style={{ zIndex: 0, pointerEvents: "none" }}>
        {backgroundType === "image" && (
            <>
              {/* 底层旧图，永远保持显示 */}
              {currentSrc && (
                  <div
                      className="absolute inset-0 w-full h-full rounded-xl"
                      style={{
                        ...commonImageStyle,
                        backgroundImage: `url(${currentSrc})`,
                        opacity: 1,
                      }}
                  />
              )}

              {/* 上层新图，从透明淡入 */}
              {nextSrc && (
                  <div
                      className="absolute inset-0 w-full h-full rounded-xl"
                      style={{
                        ...commonImageStyle,
                        backgroundImage: `url(${nextSrc})`,
                        opacity: showNext ? 1 : 0,
                        transition: "opacity 1.6s ease-in-out",
                      }}
                  />
              )}
            </>
        )}

        {backgroundType === "video" && videoSrc && !videoLoadError && (
            <video
                ref={videoRef}
                autoPlay
                loop
                muted={!videoStartSound}
                playsInline
                preload="auto"
                onError={onVideoError}
                onLoadedData={onVideoLoadedData}
                className="absolute inset-0 w-full h-full object-cover rounded-xl"
                style={{
                  ...backgroundBlurStyle,
                  zIndex: 0,
                  pointerEvents: "none",
                }}
            >
              <source src={videoSrc} type="video/mp4" />
            </video>
        )}

        <div
            className="absolute inset-0 background-overlay rounded-xl"
            style={{
              ...overlayStyle,
              pointerEvents: "none",
              zIndex: 1,
            }}
        />
      </div>
  );
}
