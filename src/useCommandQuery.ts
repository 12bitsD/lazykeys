import { useEffect, useRef, useState } from "react";
import type { Dispatch, RefObject, SetStateAction } from "react";
import { api } from "./api";
import type { SearchResult } from "./types";

/** 防抖查询的依赖集合：查询文本、请求序号和结果状态写入口。 */
type QueryEffectOptions = {
  query: string;
  searchSequence: RefObject<number>;
  setResults: Dispatch<SetStateAction<SearchResult[]>>;
  setSearching: Dispatch<SetStateAction<boolean>>;
  reportError: (error: unknown) => void;
};

/** 防抖查询只接受最新序号的结果，旧响应不能覆盖新输入。 */
function useDebouncedQuery({
  query,
  searchSequence,
  setResults,
  setSearching,
  reportError,
}: QueryEffectOptions) {
  useEffect(() => {
    const sequence = ++searchSequence.current;
    setSearching(true);
    const timer = window.setTimeout(async () => {
      try {
        const items = await api.searchCommands(query);
        if (sequence === searchSequence.current) {
          setResults(items);
        }
      } catch (error) {
        if (sequence === searchSequence.current) {
          reportError(error);
        }
      } finally {
        if (sequence === searchSequence.current) {
          setSearching(false);
        }
      }
    }, 100);
    return () => window.clearTimeout(timer);
  }, [query]);
}

/** 查询数据源：输入、最新请求门控与主动刷新。 */
export function useCommandQuery(reportError: (error: unknown) => void) {
  const [results, setResults] = useState<SearchResult[]>([]);
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(true);
  const searchSequence = useRef(0);
  const queryInputRef = useRef<HTMLInputElement>(null);
  useDebouncedQuery({
    query,
    searchSequence,
    setResults,
    setSearching,
    reportError,
  });

  /** 新输入立即使旧结果失效，再由防抖任务查询最新值。 */
  function updateQuery(value: string) {
    searchSequence.current += 1;
    setQuery(value);
    setResults([]);
    setSearching(true);
  }

  /** 回到完整词典，并保留继续输入所需的焦点。 */
  function clearQuery() {
    updateQuery("");
    queryInputRef.current?.focus();
  }

  /** 外部数据变化后主动重查，旧响应仍不能覆盖新输入。 */
  async function refresh() {
    const sequence = ++searchSequence.current;
    const items = await api.searchCommands(query);
    if (sequence === searchSequence.current) {
      setResults(items);
    }
  }

  return {
    query,
    results,
    searching,
    queryInputRef,
    updateQuery,
    clearQuery,
    refresh,
  };
}
