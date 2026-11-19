local M = {}

-- Nerd Fonts nf-md-alpha_o_circle. It occupies the same terminal cell as nf-md-language_rust.
local icon = { glyph = "󰰖", hl = "OxiFileIcon" }

function M.mini_options(opts)
  opts.extension = opts.extension or {}
  opts.filetype = opts.filetype or {}
  opts.extension.oxi = icon
  opts.filetype.oxi = icon
end

local function highlight()
  local light = vim.o.background == "light"
  vim.api.nvim_set_hl(0, icon.hl, {
    fg = light and "#50795E" or "#9AC5A3",
    ctermfg = light and 65 or 108,
  })
end

function M.setup()
  highlight()
  vim.api.nvim_create_autocmd("ColorScheme", {
    group = vim.api.nvim_create_augroup("OxiFileIcon", { clear = true }),
    callback = highlight,
  })
end

return M
