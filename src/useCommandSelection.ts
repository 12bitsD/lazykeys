import { useEffect, useMemo, useState } from "react";
import type { KeyboardEvent } from "react";
import type { CommandItem, SearchResult } from "./types";
import type { useCommandQuery } from "./useCommandQuery";

/** 查询层输出的事实集合，供可见选择与键盘导航共同消费。 */
type QueryState = ReturnType<typeof useCommandQuery>;

/** 一次键盘导航所需的查询事实、可见候选与选中状态写入口。 */
type NavigationOptions = {
  queryState: QueryState;
  displayedResults: SearchResult[];
  selected?: CommandItem;
  setSelectedId: (id: string) => void;
};

/** 计算键盘目标行，不在这里处理查询或数据请求。 */
function nextIndex(key: string, current: number, count: number) {
  if (key === "Home") {
    return 0;
  }
  if (key === "End") {
    return count - 1;
  }
  if (key === "ArrowDown") {
    return Math.min(count - 1, current + 1);
  }
  return Math.max(0, current - 1);
}

/** 键盘事件只影响清空或可见候选，不直接修改查询结果。 */
function navigateQuery(
  event: KeyboardEvent<HTMLInputElement>,
  { queryState, displayedResults, selected, setSelectedId }: NavigationOptions,
) {
  if (event.key === "Escape") {
    if (queryState.query) {
      event.preventDefault();
      queryState.clearQuery();
    }
    return;
  }
  if (queryState.searching || displayedResults.length === 0) {
    return;
  }
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
    return;
  }
  event.preventDefault();
  const current = displayedResults.findIndex(
    (result) => result.item.id === selected?.id,
  );
  const index = nextIndex(event.key, current, displayedResults.length);
  const id = displayedResults[index].item.id;
  setSelectedId(id);
  document.getElementById(`command-option-${id}`)?.scrollIntoView({
    block: "nearest",
  });
}

/** 可见候选与选中项：筛选后，详情和键盘导航读取同一集合。 */
export function useCommandSelection(queryState: QueryState) {
  const [readOnly, setReadOnly] = useState(false);
  const [selectedId, setSelectedId] = useState("");
  const displayedResults = useMemo(
    () => readOnly
      ? queryState.results.filter((result) => result.item.risk === "read")
      : queryState.results,
    [queryState.results, readOnly],
  );
  const selected = useMemo(
    () => displayedResults.find(
      (result) => result.item.id === selectedId,
    )?.item ?? displayedResults[0]?.item,
    [displayedResults, selectedId],
  );

  useEffect(() => {
    setSelectedId(selected?.id ?? "");
  }, [selected]);

  /** 把键盘导航委托给纯事件适配器，hook 只持有选择与筛选状态。 */
  function handleQueryKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    navigateQuery(event, {
      queryState,
      displayedResults,
      selected,
      setSelectedId,
    });
  }

  return {
    displayedResults,
    selected,
    readOnly,
    setReadOnly,
    setSelectedId,
    handleQueryKeyDown,
  };
}
