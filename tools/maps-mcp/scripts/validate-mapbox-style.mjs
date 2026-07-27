import { validate } from "@mapbox/mapbox-gl-style-spec";

let input = "";
for await (const chunk of process.stdin) input += chunk;

try {
  const style = JSON.parse(input);
  const diagnostics = validate(style).map((diagnostic) => ({
    severity:
      diagnostic?.constructor?.name === "ValidationWarning" ? "warning" : "error",
    message: String(diagnostic?.message ?? diagnostic),
  }));
  process.stdout.write(JSON.stringify({ diagnostics }));
} catch (error) {
  process.stdout.write(
    JSON.stringify({
      diagnostics: [
        {
          severity: "error",
          message:
            error instanceof Error ? error.message : "Mapbox style validation failed",
        },
      ],
    }),
  );
}
