interface Context {
	options: { listName: string; mode: string; buildType: string }
	fileType: { getId(path: string): string }
}

export default ({ options, fileType }: Context) => {
	const seen: string[] = []
	return {
		include() {
			return [[options.listName, { isVirtual: true }]]
		},
		transformPath(filePath: string | null) {
			if (filePath) seen.push(filePath)
		},
		read(filePath: string) {
			if (filePath === options.listName) return {}
		},
		finalizeBuild(filePath: string) {
			if (filePath !== options.listName) return
			const files = seen.map((path) => [path, fileType.getId(path)])
			return JSON.stringify({ mode: options.mode, buildType: options.buildType, files }, null, 2)
		},
	}
}
