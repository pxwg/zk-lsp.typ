-- Hook: set checklist-status = "done" for archived or legacy notes.
--
-- When a note has `relation = "archived"` or `relation = "legacy"`,
-- it is considered complete regardless of its checklist items.

function run(note)
  local r = note.metadata["relation"]
  if r == "archived" or r == "legacy" then
    return { metadata = { ["checklist-status"] = "done" } }
  end
  return {}
end
