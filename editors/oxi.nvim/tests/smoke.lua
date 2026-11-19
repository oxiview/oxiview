-- Run from the oxiview checkout with the user's LazyVim configuration.
local function check_icons(base)
  local circle_o = vim.fn.nr2char(0xF0C16)
  local group = "OxiFileIcon"
  local function expect_icon(glyph, hl)
    assert(glyph == circle_o and hl == group, "expected the .oxi circle O")
  end
  expect_icon(Snacks.util.icon("oxi", "filetype"))
  expect_icon(require("mini.icons").get("extension", "oxi"))
  expect_icon(require("nvim-web-devicons").get_icon("button_face.oxi"))
  local rust, rust_hl = Snacks.util.icon("lib.rs", "file")
  assert(rust ~= circle_o and rust_hl ~= group)
  assert(vim.fn.strdisplaywidth(circle_o) == vim.fn.strdisplaywidth(rust))

  vim.cmd.colorscheme(vim.g.colors_name)
  local color = vim.api.nvim_get_hl(0, { name = group, link = false }).fg
  assert(color == (vim.o.background == "light" and 0x50795E or 0x9AC5A3))

  local explorer = Snacks.explorer({ cwd = base, follow_file = false })
  local function rendered(filename, glyph, hl)
    local buf = explorer.list.win.buf
    if not buf or not vim.api.nvim_buf_is_valid(buf) then return false end
    for row, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, false)) do
      if line:find(filename, 1, true) then
        local marks = vim.api.nvim_buf_get_extmarks(buf, -1, { row - 1, 0 }, { row - 1, -1 }, { details = true })
        for _, mark in ipairs(marks) do
          for _, chunk in ipairs(mark[4].virt_text or {}) do
            if chunk[1]:find(glyph, 1, true) and chunk[2] == hl then return true end
          end
        end
      end
    end
    return false
  end
  assert(vim.wait(5000, function()
    return rendered("button_face.oxi", circle_o, group) and rendered("lib.rs", rust, rust_hl)
  end), "explorer rows did not render the .oxi and Rust icons")
  explorer:close()
end

local function check()
  local function attached()
    local buf = vim.api.nvim_get_current_buf()
    assert(vim.bo[buf].filetype == "oxi", "expected oxi filetype")
    assert(vim.wait(15000, function()
      local c = vim.lsp.get_clients({ bufnr = buf, name = "oxiview" })[1]
      return c and c.initialized
    end), "oxiview did not attach to " .. vim.api.nvim_buf_get_name(buf))
    return vim.lsp.get_clients({ bufnr = buf, name = "oxiview" })[1], buf
  end

  local function position(buf, needle, offset)
    for i, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, false)) do
      local start = line:find(needle, 1, true)
      if start then
        return { line = i - 1, character = vim.str_utfindex(line, "utf-16", start - 1 + offset) }
      end
    end
    error("missing source text: " .. needle)
  end

  local function request(client, buf, method, pos)
    local args = { textDocument = { uri = vim.uri_from_bufnr(buf) }, position = pos }
    local reply = assert(client:request_sync("textDocument/" .. method, args, 10000, buf))
    assert(not reply.err, vim.inspect(reply.err))
    return reply.result
  end

  local client, screen = attached()
  assert(client.offset_encoding == "utf-16")
  for _, key in ipairs({ "gd", "gr", "gI", "gD", "gy", "K" }) do
    assert(not vim.tbl_isempty(vim.fn.maparg(key, "n", false, true)), "missing LazyVim mapping " .. key)
  end
  local call = position(screen, "<game_panel", 2)
  local highlight = vim.fn.synIDattr(vim.fn.synID(call.line + 1, call.character + 1, 1), "name")
  assert(highlight == "oxiTag", "expected markup highlight, got " .. highlight)
  local target = request(client, screen, "definition", call)
  assert(target[1].uri:match("/board%.oxi$"), vim.inspect(target))
  assert(request(client, screen, "hover", call).contents.value:find("view game_panel", 1, true))
  local refs = assert(client:request_sync("textDocument/references", {
    textDocument = { uri = vim.uri_from_bufnr(screen) }, position = call, context = { includeDeclaration = true },
  }, 10000, screen))
  assert(vim.iter(refs.result):any(function(ref) return ref.uri:match("/board%.oxi$") end))
  local completions = request(client, screen, "completion", position(screen, "<Node style=", 6))
  assert(vim.iter(completions.items):any(function(item) return item.label == "bind_style" end))
  assert(vim.iter(request(client, screen, "documentSymbol")):any(function(item) return item.name == "screen" end))

  local base = vim.fs.dirname(vim.api.nvim_buf_get_name(screen))
  vim.api.nvim_win_set_cursor(0, { call.line + 1, call.character })
  vim.api.nvim_feedkeys("gd", "mx", false)
  assert(vim.wait(10000, function()
    return vim.api.nvim_buf_get_name(0) == base .. "/board.oxi"
  end), "gd did not open the board component")
  local board = vim.api.nvim_get_current_buf()
  local style = position(board, "style={overlay_surface()}", 1)
  local function board_key(key)
    vim.cmd.buffer(board)
    vim.api.nvim_win_set_cursor(0, { style.line + 1, style.character })
    vim.api.nvim_feedkeys(key, "mx", false)
  end
  for _, key in ipairs({ "gd", "gI", "gD", "gy" }) do
    board_key(key)
    assert(vim.wait(5000, function()
      local path = vim.api.nvim_buf_get_name(0)
      local text = vim.api.nvim_get_current_line()
      if key == "gy" then return path:match("oxiview%-style/src/lib%.rs$") and text:find("struct Style", 1, true) end
      return path:match("oxiview%-core/src/ui/view%.rs$") and text:find("fn style(", 1, true)
    end), key .. " returned the wrong target for Node.style")
  end
  board_key("gr")
  assert(vim.wait(10000, function()
    for _, picker in ipairs(Snacks.picker.get()) do
      for _, item in ipairs(picker:items()) do
        if item.file == base .. "/board.oxi" then return true end
      end
    end
    for _, item in ipairs(vim.fn.getqflist()) do
      if vim.api.nvim_buf_get_name(item.bufnr) == base .. "/board.oxi" then return true end
    end
    return false
  end), "gr did not list the Node.style references")
  for _, picker in ipairs(Snacks.picker.get()) do picker:close() end
  vim.cmd.cclose()
  vim.cmd.edit(vim.fn.fnameescape(base .. "/button_face.oxi"))
  local fragment_client, fragment = attached()
  assert(fragment_client.id == client.id, "expected one client per Cargo project")
  local capture = request(client, fragment, "definition", position(fragment, "{title}", 2))
  assert(capture[1].uri:match("/controls%.rs$"), vim.inspect(capture))

  -- Exercise unsaved edits with the real client, then discard the in-memory text.
  vim.api.nvim_buf_set_lines(fragment, 0, -1, false, { "<Node>", "  <Node/>", "</Wrong>" })
  assert(vim.wait(5000, function()
    return vim.iter(vim.diagnostic.get(fragment)):any(function(d)
      return d.source == "oxiview" and d.lnum == 2 and d.message:find("closing tag")
    end)
  end), "missing positioned diagnostic")
  vim.api.nvim_buf_set_lines(fragment, 0, -1, false, { "<Node/>" })
  assert(vim.wait(5000, function() return #vim.diagnostic.get(fragment) == 0 end), "stale diagnostic")
  check_icons(base)
  print(("LazyVim .oxi acceptance passed (%s): explorer icons, Node.style gd/gr/gI/gD/gy, component navigation, hover, completion, symbols, captures and live diagnostics"):format(client.config.cmd[1]))
end

local ok, err = xpcall(check, debug.traceback)
if not ok then
  io.stderr:write(err .. "\n")
  vim.cmd("cquit! 1")
end
vim.cmd("qa!")
