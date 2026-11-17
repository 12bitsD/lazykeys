local M = {
  channel = nil,
  config = {},
  pending = "",
  failed = false,
  generation = 0,
  result_generation = 0,
  last_input = "",
  redact_next = false,
  focused = true,
}

local function clear_pending()
  M.pending = ""
  M.redact_next = false
end

local function fail(error)
  if not M.failed then
    M.failed = true
    clear_pending()
    vim.schedule(function()
      vim.notify("LazyKeys companion 已停用：" .. tostring(error), vim.log.levels.WARN)
    end)
  end
end

local function safe(callback)
  return function(...)
    if M.failed then
      return
    end
    local ok, error = pcall(callback, ...)
    if not ok then
      fail(error)
    end
  end
end

local function trim_last_key(value)
  if value == "" then
    return ""
  end
  local special = value:match("^(.*)<[^>]+>$")
  if special then
    return special
  end
  return vim.fn.strcharpart(value, 0, vim.fn.strchars(value) - 1)
end

local function translated_key(typed)
  local key = vim.fn.keytrans(typed)
  if key == "<Space>" and vim.g.mapleader == " " then
    return "<leader>"
  end
  return key
end

local function arm_result_timer()
  M.result_generation = M.result_generation + 1
  local result_generation = M.result_generation
  vim.defer_fn(function()
    if result_generation == M.result_generation then
      M.last_input = ""
      M.emit()
    end
  end, M.config.result_duration or 1400)
end

local function complete_pending()
  if M.pending == "" then
    return
  end
  M.last_input = M.pending
  clear_pending()
  arm_result_timer()
  M.emit()
end

local function arm_clear_timer()
  M.generation = M.generation + 1
  local generation = M.generation
  vim.defer_fn(function()
    if generation == M.generation then
      complete_pending()
    end
  end, 1400)
end

local function has_longer_mapping(pending, mode)
  local function normalized(lhs)
    local value = lhs:lower()
    if vim.g.mapleader == " " then
      value = value:gsub("<space>", "<leader>")
    end
    return value
  end
  local expected = normalized(pending)
  local function longer(mappings)
    for _, map in ipairs(mappings) do
      local lhs = normalized(map.lhs)
      if #lhs > #expected and lhs:sub(1, #expected) == expected then
        return true
      end
    end
    return false
  end
  return longer(vim.api.nvim_get_keymap(mode)) or longer(vim.api.nvim_buf_get_keymap(0, mode))
end

M.has_longer_mapping = has_longer_mapping

local function command_pending()
  local command_type = vim.fn.getcmdtype()
  if command_type ~= ":" then
    return command_type
  end
  local command = vim.fn.getcmdline():match("^%s*([%w_]+)") or ""
  return ":" .. command
end

local function on_key(_, typed)
  if typed == "" then
    return
  end
  local mode = vim.fn.mode(1)
  M.result_generation = M.result_generation + 1
  M.last_input = ""
  local key = translated_key(typed)
  if key == "<Esc>" then
    clear_pending()
    M.last_input = ""
    M.result_generation = M.result_generation + 1
    M.emit()
    return
  end
  if mode:sub(1, 1) == "i" or mode:sub(1, 1) == "t" then
    clear_pending()
    return
  end
  if mode:sub(1, 1) == "c" then
    M.pending = command_pending()
    M.redact_next = false
    arm_clear_timer()
    M.emit()
    return
  end
  if M.redact_next then
    M.pending = M.pending .. "<redacted>"
    M.redact_next = false
  elseif key == "<BS>" then
    M.pending = trim_last_key(M.pending)
  elseif mode:sub(1, 2) == "no" or M.pending ~= "" then
    M.pending = M.pending .. key
  else
    M.pending = key
  end
  if not M.redact_next and M.pending:match("[r f F t T m ' ` \" @ q]$") then
    M.redact_next = true
  end
  arm_clear_timer()
  M.emit()
  vim.schedule(function()
    if M.pending ~= "" and vim.fn.mode(1) == "n" then
      local mapping = vim.fn.maparg(M.pending, "n", false, true)
      if type(mapping) == "table" and mapping.lhs and not has_longer_mapping(M.pending, "n") then
        complete_pending()
      end
    end
  end)
end

function M.emit(event_name)
  if event_name == "FocusGained" then
    M.focused = true
  elseif event_name == "FocusLost" then
    M.focused = false
  end
  if vim.in_fast_event() then
    vim.schedule(function()
      M.emit(event_name)
    end)
    return
  end
  if M.failed or not M.channel then
    return
  end
  local session_id = M.config.session_id or tostring(vim.fn.getpid())
  local profile_id = M.config.profile_id or vim.env.NVIM_APPNAME or "default"
  local last_input = M.config.show_history == false and "" or M.last_input
  local context = require("lazykeys.context").current(M.pending, session_id, profile_id, M.focused, last_input)
  vim.rpcnotify(M.channel, "lazykeys_context", vim.json.encode(context))
end

function M.emit_mappings()
  if vim.in_fast_event() then
    vim.schedule(M.emit_mappings)
    return
  end
  if M.failed or not M.channel then
    return
  end
  vim.rpcnotify(M.channel, "lazykeys_mappings", vim.json.encode(M.export_keymaps()))
end

function M.register(channel, session_id, profile_id)
  M.channel = channel
  M.config.session_id = session_id or M.config.session_id
  M.config.profile_id = profile_id or M.config.profile_id
  M.emit()
  return true
end

function M.export_keymaps()
  local modes = { "n", "i", "v", "x", "o", "c", "t" }
  local result = {}
  for _, mode in ipairs(modes) do
    for _, map in ipairs(vim.api.nvim_get_keymap(mode)) do
      table.insert(result, {
        mode = mode,
        lhs = map.lhs,
        rhs = type(map.rhs) == "string" and map.rhs or "",
        desc = map.desc or "",
        scope = "global",
      })
    end
    for _, map in ipairs(vim.api.nvim_buf_get_keymap(0, mode)) do
      table.insert(result, {
        mode = mode,
        lhs = map.lhs,
        rhs = type(map.rhs) == "string" and map.rhs or "",
        desc = map.desc or "",
        scope = "buffer",
      })
    end
  end
  return result
end

local function setup_autocmds()
  local group = vim.api.nvim_create_augroup("LazyKeysCompanion", { clear = true })
  vim.api.nvim_create_autocmd({
    "ModeChanged",
    "BufEnter",
    "WinEnter",
    "FileType",
    "FocusGained",
    "FocusLost",
    "CmdlineChanged",
    "VimLeavePre",
  }, {
    group = group,
    callback = safe(function(event)
      if event.event == "ModeChanged" and M.pending ~= "" then
        if event.match:match("^no") or event.match:match("^c") then
          complete_pending()
        elseif event.match:match("^i") or event.match:match("^t") then
          clear_pending()
          M.emit(event.event)
        end
      elseif (event.event == "BufEnter" or event.event == "WinEnter" or event.event == "FileType") and M.pending ~= "" then
        complete_pending()
      end
      if event.event == "BufEnter" or event.event == "FileType" then
        M.emit_mappings()
      end
      M.emit(event.event)
    end),
  })
  vim.api.nvim_create_autocmd("User", {
    group = group,
    pattern = { "VeryLazy", "LazyDone" },
    callback = safe(function(event)
      M.emit_mappings()
      M.emit(event.event)
    end),
  })
  vim.api.nvim_create_autocmd("OptionSet", {
    group = group,
    pattern = { "mapleader", "maplocalleader" },
    callback = safe(function(event)
      M.emit_mappings()
      M.emit(event.event)
    end),
  })
end

local function setup_commands()
  vim.api.nvim_create_user_command("LazyKeys", function()
    local status = M.failed and "disabled" or (M.channel and "connected" or "waiting")
    vim.notify("LazyKeys: " .. status .. " · " .. (M.config.profile_id or "default"))
  end, {})
  vim.api.nvim_create_user_command("LazyKeysReconnect", function()
    M.failed = false
    M.emit()
    M.emit_mappings()
  end, {})
end

function M.setup(options)
  M.config = vim.tbl_deep_extend("force", {
    session_id = tostring(vim.fn.getpid()),
    profile_id = vim.env.NVIM_APPNAME or "default",
  }, M.config, options or {})
  M.failed = false
  setup_autocmds()
  setup_commands()
  vim.on_key(safe(on_key), vim.api.nvim_create_namespace("lazykeys"))
  M.emit()
end

return M
