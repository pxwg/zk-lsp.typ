-- Hook: backfill config-declared metadata keys when a note is missing them.
--
-- Intended usage:
--   1. Keep zk-lsp's built-in hooks enabled.
--   2. Add this hook in zk-lsp.toml / config.toml.
--   3. On save, zk-lsp format will patch any missing config-declared
--      metadata keys using their configured default values.
--
-- This hook relies on two zk-lsp hook inputs:
--   note.metadata         -- current parsed TOML metadata
--   note.metadata_fields  -- config-declared fields with path/kind/default
--
-- The hook only emits missing keys. Existing values are left untouched.

local function lookup_path(tbl, path)
  local current = tbl
  for segment in path:gmatch("[^.]+") do
    if type(current) ~= "table" then
      return nil
    end
    current = current[segment]
    if current == nil then
      return nil
    end
  end
  return current
end

function run(note)
  local patch = {}

  for _, field in ipairs(note.metadata_fields or {}) do
    if lookup_path(note.metadata, field.path) == nil then
      patch[field.path] = field.default
    end
  end

  if next(patch) == nil then
    return {}
  end

  return { metadata = patch }
end
