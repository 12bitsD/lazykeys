local M = {}

local function buffer_key(win)
  return vim.api.nvim_win_get_buf(win)
end

local function has_window_with_filetype(target)
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    if vim.bo[buffer_key(win)].filetype == target then
      return true
    end
  end
  return false
end

local function find_exit_key(kind, mode)
  local descriptions = { "cancel", "close", "quit", "close window" }
  local maps = vim.api.nvim_buf_get_keymap(0, mode)
  for _, map in ipairs(maps) do
    local desc = type(map.desc) == "string" and map.desc:lower() or ""
    for _, expected in ipairs(descriptions) do
      if desc:find(expected, 1, true) then
        return map.lhs
      end
    end
  end
  if (kind == "lazy" or kind == "mason") and mode == "n" then
    return "q"
  end
  if kind == "snacksPreview" and mode == "n" then
    return "q"
  end
  return nil
end

function M.current(pending, session_id, profile_id, focused, last_input)
  local mode = vim.fn.mode(1)
  local filetype = vim.bo.filetype
  local buftype = vim.bo.buftype
  local kind = "normal"
  local title = "当前编辑窗口"

  if filetype == "which-key" then
    kind = "whichKey"
    title = "which-key 命令菜单"
  elseif filetype == "snacks_picker_input" then
    kind = "snacksInput"
    title = "Snacks 搜索输入框"
  elseif filetype == "snacks_picker_list" then
    kind = "snacksList"
    title = "Snacks 搜索结果"
  elseif filetype == "snacks_picker_preview" then
    kind = "snacksPreview"
    title = "Snacks 预览"
  elseif filetype == "lazy" then
    kind = "lazy"
    title = "lazy.nvim 插件管理"
  elseif filetype == "mason" then
    kind = "mason"
    title = "Mason 外部工具管理"
  elseif filetype == "help" or buftype == "help" then
    kind = "help"
    title = "Neovim 帮助"
  elseif buftype == "quickfix" then
    kind = "quickfix"
    title = "Quickfix 列表"
  elseif buftype == "terminal" or mode == "t" then
    kind = "terminal"
    title = "Neovim 内置终端"
  elseif filetype:find("snacks") or filetype:find("trouble") then
    kind = "unknown"
    title = filetype ~= "" and filetype or "插件页面"
  elseif has_window_with_filetype("which-key") then
    kind = "whichKey"
    title = "which-key 命令菜单"
  end

  return {
    sessionId = session_id or "",
    profileId = profile_id or "default",
    mode = mode,
    pending = pending,
    lastInput = last_input or "",
    kind = kind,
    title = title,
    cwd = vim.fn.getcwd(),
    filetype = filetype,
    bufferModified = vim.bo.modified,
    exitKey = find_exit_key(kind, mode),
    focused = focused ~= false,
  }
end

return M
