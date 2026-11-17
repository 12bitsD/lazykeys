local ok, lazykeys = pcall(require, "lazykeys")
if ok then
  lazykeys.setup()
else
  vim.schedule(function()
    vim.notify("LazyKeys companion 加载失败：" .. tostring(lazykeys), vim.log.levels.WARN)
  end)
end
