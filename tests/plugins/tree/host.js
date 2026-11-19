// A tree whose folders give their rows when they are opened. What one turn shows (the
// panel's rows and the count beside them) is shown together.
const folder = (id) => ({ id, label: id, lazy: true });

const show = async () => {
  await lume.panels.set("files", [
    folder("src"),
    folder("docs"),
    { id: "README", label: "README" },
  ]);
};

lume.commands.register("show", show);

lume.panels.onExpand("files", (event) => {
  lume.log.info("asked for", event.path.join("/"));
  if (event.row === "docs") return; // Never answered.
  const rows =
    event.path.length === 1
      ? [folder("nested"), { id: "main.rs", label: "main.rs" }, { id: "lib.rs", label: "lib.rs" }]
      : [{ id: "deep.rs", label: "deep.rs" }];
  lume.panels.setChildren("files", event.path, rows).catch((error) => lume.log.error(error));
});

void show();
