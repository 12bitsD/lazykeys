import type { useCommandSearch } from "../useCommandSearch";

/** 查询面板读取的完整搜索状态机，不另建候选或选中副本。 */
type Search = ReturnType<typeof useCommandSearch>;

/** 结果统计只描述当前匹配与筛选，不参与候选计算。 */
function resultSummary(search: Search) {
  if (search.searching) {
    return "正在搜索…";
  }
  if (search.readOnly) {
    const visible = search.displayedResults.length;
    return `${visible} 条只读结果 · 共 ${search.results.length} 条匹配`;
  }
  return `${search.results.length} 条结果 · 来自当前配置与本地词典`;
}

/** 查询输入与筛选：键盘事件直接交给搜索状态机。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function QueryInput({ search }: { search: Search }) {
  const activeId = !search.searching && search.selected
    ? `command-option-${search.selected.id}`
    : undefined;
  return (
    <>
      <label htmlFor="command-query">输入要做什么，或直接输入按键</label>
      <input
        ref={search.queryInputRef}
        id="command-query"
        role="combobox"
        value={search.query}
        onChange={(event) => search.updateQuery(event.target.value)}
        onKeyDown={search.handleQueryKeyDown}
        placeholder="例如：查找文件 / 怎么退出搜索 / diw"
        autoComplete="off"
        aria-autocomplete="list"
        aria-controls="command-results"
        aria-expanded={!search.searching && search.displayedResults.length > 0}
        aria-activedescendant={activeId}
        aria-describedby="query-keyboard-hint"
      />
      <div className="query-keyboard-hint" id="query-keyboard-hint">
        <span><kbd>↑</kbd><kbd>↓</kbd> 选择</span>
        <span><kbd>Esc</kbd> 清空</span>
      </div>
      <div className="query-filter">
        <label htmlFor="read-only-filter">
          <input
            id="read-only-filter"
            type="checkbox"
            checked={search.readOnly}
            onChange={(event) => search.setReadOnly(event.target.checked)}
          />
          只看只读命令
        </label>
      </div>
      <div
        className="result-count"
        aria-live={search.searching ? "off" : "polite"}
      >
        {resultSummary(search)}
      </div>
    </>
  );
}

/** 候选列表只渲染可见集合，点击与键盘选择共用选中状态。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function CandidateList({ search }: { search: Search }) {
  return (
    <div
      className="candidate-list"
      id="command-results"
      role="listbox"
      aria-label="命令候选"
      hidden={search.searching || search.displayedResults.length === 0}
    >
      {search.displayedResults.map((result) => (
        <button
          key={result.item.id}
          id={`command-option-${result.item.id}`}
          role="option"
          tabIndex={-1}
          aria-selected={search.selected?.id === result.item.id}
          className={`candidate ${
            search.selected?.id === result.item.id ? "selected" : ""
          }`}
          onClick={() => search.setSelectedId(result.item.id)}
        >
          <span>{result.item.title}</span>
          <kbd>{result.item.lhs}</kbd>
        </button>
      ))}
    </div>
  );
}

/** 空结果文案区分加载、筛选无结果和词典无匹配。 */
function emptyMessage(search: Search) {
  if (search.searching) {
    return "正在更新结果…";
  }
  if (search.readOnly && search.results.length > 0) {
    return "当前搜索没有只读命令，可关闭筛选查看全部。";
  }
  return "没有匹配结果。可以换一个中文动作或按键。";
}

/** 空状态提供最短的恢复动作，不引入额外导航。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function EmptyAction({ search }: { search: Search }) {
  if (!search.searching && search.readOnly && search.results.length > 0) {
    return (
      <button
        className="secondary"
        onClick={() => {
          search.setReadOnly(false);
          search.queryInputRef.current?.focus();
        }}
      >
        查看全部 {search.results.length} 条结果
      </button>
    );
  }
  if (!search.searching && search.query) {
    return (
      <button className="secondary" onClick={search.clearQuery}>
        清空搜索
      </button>
    );
  }
  return null;
}

/** 搜索面板组合输入、候选与空状态，不直接操作后端。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function QueryPanel({ search }: { search: Search }) {
  const empty = search.searching || search.displayedResults.length === 0;
  return (
    <div className="query-panel">
      <QueryInput search={search} />
      <CandidateList search={search} />
      {empty && (
        <div className="candidate-list empty-results">
          <div className="empty-state">
            <p>{emptyMessage(search)}</p>
            <EmptyAction search={search} />
          </div>
        </div>
      )}
    </div>
  );
}
