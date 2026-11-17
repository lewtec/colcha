package modot

modules: {
	old: {
		from: "core:place"
		config: {
			items: {
				"/.config/legacy": "legacy-src"
			}
			steps: {
				"10-require": {
					op: "require"
					patterns: {
						"*.txt": "*"
					}
				}
			}
		}
	}
}

backup: actions: [
	{
		name: "git repo personal"
		kind: "git_repo_sync"
		src:  "/home/u/.personal"
		dst:  "remote:personal"
	},
	{
		name: "archive cache"
		kind: "archive"
		input_dir: "/home/u/.cache"
		output:    "/home/u/backup.tar"
		format:    "tar"
	},
]

lint: tools: {
	"00-go": {
		detect: {
			"00-go-mod": {
				path:   "go.mod"
				enable: true
			}
		}
		needs: {
			golangci_lint: true
		}
		cmd:    ["golangci-lint", "run"]
		output: "sarif"
	}
}
