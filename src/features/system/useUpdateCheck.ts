/** 所有更新入口共享查询键与策略；关闭自动检查仍允许用户主动检查。 */
import { useQuery } from '@tanstack/react-query';

import { isTauriRuntime, updateCheck } from '@/lib/ipc';
import {
  DEFAULT_UPDATE_AUTO_CHECK,
  UPDATE_AUTO_CHECK_KEY,
  useSettingsStore,
} from '@/stores/settingsStore';

export const UPDATE_QUERY_KEY = ['app', 'update'] as const;

export function useUpdateCheck(poll = false) {
  const loaded = useSettingsStore((state) => state.loaded);
  const automatic = useSettingsStore((state) =>
    state.getJson<boolean>(UPDATE_AUTO_CHECK_KEY, DEFAULT_UPDATE_AUTO_CHECK),
  );
  return useQuery({
    queryKey: UPDATE_QUERY_KEY,
    queryFn: updateCheck,
    enabled: isTauriRuntime() && loaded && automatic,
    staleTime: Number.POSITIVE_INFINITY,
    retry: 0,
    refetchInterval: poll && automatic ? 30 * 60 * 1000 : false,
  });
}
