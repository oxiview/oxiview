-- Link this file into LazyVim's lua/plugins directory for a local checkout.
local source = debug.getinfo(1, "S").source:sub(2)
local plugin = vim.fs.dirname(assert(vim.uv.fs_realpath(source)))
local cargo_home = vim.env.CARGO_HOME or vim.fs.joinpath(vim.uv.os_homedir(), ".cargo")
local executable = vim.fn.has("win32") == 1 and "oxiview-lsp.exe" or "oxiview-lsp"

return {
  {
    dir = plugin,
    name = "oxi.nvim",
    main = "oxi",
    lazy = false,
    opts = { cmd = { vim.fs.joinpath(cargo_home, "bin", executable) } },
  },
  {
    "nvim-mini/mini.icons",
    optional = true,
    opts = function(_, opts)
      require("oxi.icons").mini_options(opts)
    end,
  },
}
