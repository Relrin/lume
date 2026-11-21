// Writes down every session whose whole state it is told, with the pane that shows it.
lume.sessions.onEvent((report) => {
  if (report.kind === "state") lume.log.info(`heard the state of ${report.pane ?? "no pane"}`);
});
