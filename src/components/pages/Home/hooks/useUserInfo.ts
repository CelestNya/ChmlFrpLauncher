import { useState, useEffect, useRef } from "react";
import { toast } from "sonner";
import {
  fetchUserInfo,
  getStoredUser,
  saveStoredUser,
  type UserInfo,
  type StoredUser,
} from "@/services/api";
import { performLogout } from "@/services/logout";
import { homePageCache } from "../cache";

function getInitialUserInfo(): UserInfo | null {
  const storedUser = getStoredUser();
  if (
    storedUser?.usertoken &&
    homePageCache.userInfo &&
    homePageCache.userInfo.usertoken === storedUser.usertoken
  ) {
    return homePageCache.userInfo;
  }
  return null;
}

export function useUserInfo(
  user: StoredUser | null | undefined,
  onUserChange?: (user: StoredUser | null) => void,
) {
  const [userInfo, setUserInfo] = useState<UserInfo | null>(getInitialUserInfo);
  const onUserChangeRef = useRef(onUserChange);
  const isFirstLoadRef = useRef(!getInitialUserInfo());

  useEffect(() => {
    onUserChangeRef.current = onUserChange;
  }, [onUserChange]);

  useEffect(() => {
    const loadUserInfo = async () => {
      const storedUser = getStoredUser();
      if (!storedUser?.usertoken) {
        setUserInfo(null);
        homePageCache.userInfo = null;
        homePageCache.flowData = [];
        homePageCache.signInInfo = null;
        return;
      }

      if (
        homePageCache.userInfo &&
        homePageCache.userInfo.usertoken === storedUser.usertoken
      ) {
        setUserInfo(homePageCache.userInfo);
        isFirstLoadRef.current = false;
      } else {
        if (homePageCache.userInfo?.usertoken !== storedUser.usertoken) {
          homePageCache.flowData = [];
          homePageCache.signInInfo = null;
        }
        isFirstLoadRef.current = true;
      }

      try {
        const data = await fetchUserInfo();
        setUserInfo(data);
        homePageCache.userInfo = data;
        isFirstLoadRef.current = false;
        const currentStoredUser = getStoredUser();
        const updatedUser = {
          username: data.username,
          usergroup: data.usergroup,
          userimg: data.userimg,
          usertoken: data.usertoken,
          accessToken: currentStoredUser?.accessToken,
          refreshToken: currentStoredUser?.refreshToken,
          accessTokenExpiresAt: currentStoredUser?.accessTokenExpiresAt,
          tokenType: currentStoredUser?.tokenType,
          tunnelCount: data.tunnelCount,
          tunnel: data.tunnel,
        };
        saveStoredUser(updatedUser);
        onUserChangeRef.current?.(updatedUser);
      } catch (err) {
        const message = err instanceof Error ? err.message : "";
        const currentStoredUser = getStoredUser();
        const isAuthError =
          !currentStoredUser ||
          message.includes("登录信息已过期") ||
          message.includes("无效的登录状态") ||
          message.includes("invalid_grant");
        if (isAuthError) {
          await performLogout();
          setUserInfo(null);
          homePageCache.userInfo = null;
          homePageCache.flowData = [];
          homePageCache.signInInfo = null;
          onUserChangeRef.current?.(null);
          toast.error("登录状态已失效，请重新登录");
        }
        console.error("获取用户信息失败", err);
      }
    };
    loadUserInfo();
  }, [user?.usertoken]);

  return {
    userInfo,
  };
}
