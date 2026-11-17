import { useCommandQuery } from "./useCommandQuery";
import { useCommandSelection } from "./useCommandSelection";

/** 命令搜索的组合接口：查询事实与可见选择各自独立演进。 */
export function useCommandSearch(reportError: (error: unknown) => void) {
  const queryState = useCommandQuery(reportError);
  const selection = useCommandSelection(queryState);
  return { ...queryState, ...selection };
}
