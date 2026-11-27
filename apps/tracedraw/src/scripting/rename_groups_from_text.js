// Rename groups in the selection, or on the active layer if nothing is selected.
(() => {
  const selection = ActiveDocument.Selection;
  const layer = selection.Count ? null : ActiveDocument.ActiveLayer;
  const roots = selection.Count ? selection.Shapes : layer.Locked ? [] : layer.Shapes.All.Shapes;
  const scanned = new Map();
  let renamed = 0;
  // Post-order walk without recursion. Keep at most two text references:
  // once there are two, the group is ambiguous even if one is empty.
  const stack = roots.map(shape => ({ root: true, shape }));
  while (stack.length) {
    const entry = stack.pop();
    const shape = entry.shape;
    if (scanned.has(shape.id)) continue;
    if (!entry.children) {
      if ((entry.root ? shape.Type : shape.ChildType) !== 'Group') continue;
      entry.children = shape.Children;
      stack.push(entry);
      for (const child of entry.children) {
        if (child.ChildType === 'Group') stack.push({ shape: child });
      }
      continue;
    }
    const blocks = [];
    for (const child of entry.children) {
      if (child.IsText) blocks.push(child);
      else if (child.ChildType === 'Group') {
        for (const text of scanned.get(child.id) || []) blocks.push(text);
      }
      if (blocks.length >= 2) break;
    }
    scanned.set(shape.id, blocks.slice(0, 2));
    if (blocks.length !== 1) continue;
    const name = blocks[0].Text.trim().replace(/\s+/g, ' ');
    if (name && shape.Name !== name) {
      shape.Name = name;
      renamed++;
    }
  }
  print(renamed);
})();
