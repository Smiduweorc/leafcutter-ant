export default ({ options, fileType }) => {
	const seen = new Set()
	return {
		transformPath(filePath) {
			if (filePath && filePath.endsWith('.txt')) return filePath.replace(/\.txt$/, '.md')
		},
		async finalizeBuild(filePath, fileContent) {
			seen.add(filePath)
			return undefined
		},
	}
}
