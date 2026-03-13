-- Hook: local nested checkbox propagation + checklist-status update
--
-- Local items participate in nested parent propagation.
-- Ref items do NOT participate in parent propagation, but they DO contribute
-- to checklist-status as ordinary observed checklist entries.
--
-- No cross-file information is read here.
local function get_state(checkbox)
  return checkbox.checked and "x" or " "
end

--- Build a flat list of local tree items for nested propagation only.
local function tree_items(checkboxes)
  local items = {}
  for _, cb in ipairs(checkboxes) do
    if cb.kind == "local" then
      -- Parse line_idx and indent from id: "local:{line_idx}:{indent}"
      local line_idx, indent = cb.id:match("^local:(%d+):(%d+)$")
      if line_idx then
        table.insert(items, {
          line_idx = tonumber(line_idx),
          indent = tonumber(indent),
          checked = cb.checked,
          span = cb.span,
          cb = cb,
        })
      end
    end
  end
  return items
end

--- Build a flat list of items that contribute to checklist-status.
--- Local items are included; ref items are also included as observed leaves.
local function status_items(checkboxes, propagated_tree_items)
  local items = {}
  -- Keep propagated local items first, so parent state changes are reflected.
  for _, item in ipairs(propagated_tree_items) do
    table.insert(items, item)
  end
  -- Add ref items as leaf observations.
  for _, cb in ipairs(checkboxes) do
    if cb.kind == "ref" then
      table.insert(items, {
        line_idx = nil,
        indent = nil,
        checked = cb.checked,
        span = cb.span,
        cb = cb,
        is_ref = true,
      })
    end
  end
  return items
end

--- Propagate bottom-up: if item has children, its state = all children done.
--- Returns a list of edits (only for items whose state changed).
local function propagate(items)
  local edits = {}
  -- Work from last to first so parents see updated children
  for i = #items, 1, -1 do
    local item = items[i]
    -- Find direct children: next items with strictly greater indent
    -- until we hit an item with indent <= item.indent
    local children = {}
    for j = i + 1, #items do
      if items[j].indent <= item.indent then
        break
      end
      table.insert(children, items[j])
    end
    if #children > 0 then
      local all_done = true
      for _, child in ipairs(children) do
        if not child.checked then
          all_done = false
          break
        end
      end

      local new_state = all_done and "x" or " "
      if get_state(item) ~= new_state then
        -- Update in-memory state for parent propagation
        item.checked = all_done

        -- Compute byte offset of the state character: start_byte + indent + 3
        local state_byte = item.span.start_byte + item.indent + 3
        table.insert(edits, {
          start_byte = state_byte,
          end_byte = state_byte + 1,
          text = new_state,
        })
      end
    end
  end
  return edits
end

--- Compute checklist-status from leaf local items plus ref items.
local function compute_status(items)
  local leaves = {}
  for i, item in ipairs(items) do
    if item.is_ref then
      table.insert(leaves, item)
    else
      local is_leaf = true
      if i < #items and items[i + 1].indent and item.indent and items[i + 1].indent > item.indent then
        is_leaf = false
      end
      if is_leaf then
        table.insert(leaves, item)
      end
    end
  end
  if #leaves == 0 then
    return nil
  end
  local all_done = true
  local any_done = false
  for _, leaf in ipairs(leaves) do
    if leaf.checked then
      any_done = true
    else
      all_done = false
    end
  end
  if all_done then
    return "done"
  elseif any_done then
    return "wip"
  else
    return "todo"
  end
end

function run(note)
  local tree = tree_items(note.checkboxes)
  local edits = propagate(tree)
  local items = status_items(note.checkboxes, tree)
  local status = compute_status(items)
  if #items == 0 then
    return {}
  end
  local result = { edits = edits }
  if status then
    result.metadata = { ["checklist-status"] = status }
  end
  return result
end
