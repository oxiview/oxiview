local M = {}

---Enable file detection and the native Neovim LSP client for .oxi buffers.
---@param opts? vim.lsp.Config
function M.setup(opts)
  require("oxi.icons").setup()
  vim.filetype.add({ extension = { oxi = "oxi" } })
  vim.lsp.config("oxiview", opts or {})
  vim.lsp.enable("oxiview")
end

return M
