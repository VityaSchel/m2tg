import path from "path";

const platforms: Record<string, Bun.Build.CompileTarget> = {
	"linux-x64-pre2013": "bun-linux-x64-baseline",
	"linux-x64": "bun-linux-x64-modern",
	"linux-arm64": "bun-linux-arm64-modern",
	"windows-x64-pre2013": "bun-windows-x64-baseline",
	"windows-x64": "bun-windows-x64-modern",
	"windows-arm64": "bun-windows-arm64",
	"darwin-x64-pre2013": "bun-darwin-x64-baseline",
	"darwin-x64": "bun-darwin-x64-modern",
	"darwin-arm64": "bun-darwin-arm64",
	"linux-x64-musl-pre2013": "bun-linux-x64-baseline-musl",
	"linux-x64-musl": "bun-linux-x64-modern-musl",
	"linux-arm64-musl": "bun-linux-arm64-modern-musl",
} as const;

await Promise.all(
	Object.entries(platforms).map(async ([filename, target]) => {
		await Bun.build({
			entrypoints: [path.join(__dirname, "./index.ts")],
			compile: {
				target,
				outfile: path.join(__dirname, `../m2tg-${filename}`),
			},
			
		});
	}),
);
