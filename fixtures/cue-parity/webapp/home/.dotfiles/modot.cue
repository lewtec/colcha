package modot

modules: {
	webapp: {
		input: "self:modules/webapp"
	}
}

options: webapp: {
	default: {
		browser: "zen"
		webapp:  "brave"
	}
	apps: {
		whatsapp: {
			url:         "https://web.whatsapp.com"
			extra_flags: []
		}
		meet: {
			browser:      "brave"
			url:          "https://meet.google.com"
			profile:      "work"
			desktop_name: "Meet"
			icon:         "meet"
			extra_flags:  ["--app=https://meet.google.com"]
		}
	}
}
