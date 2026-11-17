import type { ProfileActions } from "../profileActions";
import type { useAppData } from "../useAppData";
import type { DependencyStatus } from "../types";
import { ProfileEditor } from "./ProfileEditor";

/** 环境页输入：共享环境数据、全局 busy 与 Profile 动作集合。 */
type EnvironmentProps = {
  data: ReturnType<typeof useAppData>;
  busy: string;
  actions: ProfileActions;
};

/** 必需与可选依赖使用不同语义，状态颜色不是唯一提示。 */
function dependencyTone(dependency: DependencyStatus) {
  if (dependency.installed) {
    return { className: "ok", icon: "✓" };
  }
  if (dependency.required) {
    return { className: "missing", icon: "!" };
  }
  return { className: "optional", icon: "–" };
}

/** 单项依赖的只读检查结果。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function DependencyRow({ dependency }: { dependency: DependencyStatus }) {
  const tone = dependencyTone(dependency);
  return (
    <div className="dependency-row">
      <span className={`dependency-icon ${tone.className}`}>{tone.icon}</span>
      <div>
        <strong>{dependency.name}</strong>
        <small>{dependency.detail}</small>
      </div>
      <span className="dependency-version">
        {dependency.version || (dependency.required ? "必需" : "可选")}
      </span>
    </div>
  );
}

/** 同类依赖列表，不为每个条目重复布局结构。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function DependencyList({ dependencies }: {
  dependencies: DependencyStatus[];
}) {
  return (
    <div className="dependency-list">
      {dependencies.map((dependency) => (
        <DependencyRow key={dependency.id} dependency={dependency} />
      ))}
    </div>
  );
}

/** 依赖统计与分组：必需项决定是否阻断，可选项只影响增强体验。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
function Dependencies({ dependencies }: {
  dependencies: DependencyStatus[];
}) {
  const required = dependencies.filter((item) => item.required);
  const optional = dependencies.filter((item) => !item.required);
  const missing = required.filter((item) => !item.installed).length;
  const optionalReady = optional.filter((item) => item.installed).length;
  return (
    <>
      <div className="section-heading dependency-heading">
        <h2>依赖检查</h2>
        <p>
          {missing === 0
            ? `${required.length} 项必需依赖已就绪 · ${optionalReady}/${optional.length} 项可选增强已安装`
            : `${missing} 项必需依赖未满足 · 安装前需要处理`}
        </p>
      </div>
      <DependencyList dependencies={required} />
      {optional.length > 0 && (
        <details className="optional-dependencies">
          <summary>可选增强（{optionalReady}/{optional.length}）</summary>
          <DependencyList dependencies={optional} />
        </details>
      )}
    </>
  );
}

/** 环境工作区：组合 Profile 卡片与依赖检查。 */
// bits:allow(BCS010): React 组件必须采用 PascalCase
export function EnvironmentView({ data, busy, actions }: EnvironmentProps) {
  return (
    <section className="environment-section">
      <div className="section-heading">
        <h2>LazyVim Profile</h2>
        <p>使用独立 NVIM_APPNAME，不修改现有 ~/.config/nvim。</p>
      </div>
      {data.profiles.map((profile) => (
        <ProfileEditor
          key={profile.config.id}
          profile={profile}
          busy={busy}
          installLog={data.installLogs[profile.config.id]}
          actions={actions}
        />
      ))}
      <Dependencies dependencies={data.dependencies} />
    </section>
  );
}
