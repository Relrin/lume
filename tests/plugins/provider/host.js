// Doubles a number, and says which install answered.
lume.services.export("double", (args) => ({ twice: args.n * 2, from: lume.self.id }));
