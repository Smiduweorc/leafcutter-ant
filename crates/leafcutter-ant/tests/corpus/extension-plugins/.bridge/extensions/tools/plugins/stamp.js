import { describe } from './shared'
import settings, { skip } from './settings.json'

export default ({
	options,
	console,
	fileSystem,
	outputFileSystem,
	projectConfig,
	projectRoot,
	packType,
	fileType,
	targetVersion,
	hasComMojangDirectory,
	getAliases,
	getAliasesWhere,
	getFileMetadata,
	addFileDependencies,
	getOutputPath,
	compileFiles,
	jsonStringifyWithFloatFix,
}) => {
	let buildStarts = 0
	const sameHandle = {}
	return {
		async buildStart() {
			buildStarts++
			const config = await fileSystem.readJson('config.json')
			console.log('stamp sees', config.targetVersion)
		},
		include() {
			return [['BP/generated/virtual.json', { isVirtual: true }], 'BP/missing.json']
		},
		ignore(filePath) {
			return filePath.endsWith('.png')
		},
		transformPath(filePath) {
			if (filePath.endsWith(skip)) return null
			if (filePath.endsWith('.txt')) return filePath.replace(/\.txt$/, '.text')
		},
		async read(filePath, fileHandle) {
			if (filePath === 'BP/generated/virtual.json') return { virtual: true, noHandle: fileHandle === undefined }
			if (filePath === 'BP/generated/late.json') return { late: true }
			if (filePath.endsWith('throws.json')) throw new TypeError(`cannot read ${filePath}`)
			if (!filePath.endsWith('.json') || !fileHandle) return
			sameHandle[filePath] = fileHandle.getFile() === fileHandle.getFile()
			const file = await fileHandle.getFile()
			if (!file) return null
			return JSON.parse(await file.text())
		},
		load(filePath, data) {
			if (data && typeof data === 'object') data.loadedBy = describe(filePath)
		},
		registerAliases(filePath, data) {
			if (data && typeof data.identifier === 'string') return data.identifier
			if (filePath.endsWith('/a.json')) return ['alias:a', 'alias:a2']
		},
		require(filePath, data) {
			if (data && data.needs) return data.needs
		},
		async transform(filePath, data, dependencies) {
			if (!data || typeof data !== 'object' || Array.isArray(data)) return
			data.dependencies = Object.keys(dependencies)
			data.dependencyIds = Object.values(dependencies).map((dep) => dep?.identifier ?? null)
			data.aliases = getAliases(filePath)
			data.where = getAliasesWhere((alias) => alias.startsWith('alias:'))
			data.outputPath = await getOutputPath(filePath)
			data.sameHandle = sameHandle[filePath] ?? null
			data.context = {
				mode: options.mode,
				buildType: options.buildType,
				projectRoot,
				targetVersion,
				hasComMojangDirectory,
				pack: packType.getId(filePath),
				type: fileType.getId(filePath),
				packs: projectConfig.getAvailablePacks(),
				resolved: projectConfig.resolvePackPath('resourcePack', 'textures/x.png'),
				sameFs: fileSystem === outputFileSystem,
			}
			getFileMetadata(filePath).set('stamped', data.loadedBy)
			if (filePath.endsWith('/b.json')) addFileDependencies(filePath, ['BP/data/a.json'])
			if (filePath.endsWith('/oops.json')) throw new Error('transform failed on purpose')
		},
		finalizeBuild(filePath, data) {
			if (filePath.endsWith('float.json')) return jsonStringifyWithFloatFix(data, settings.floatPaths.map((pathGlob) => ({ pathGlob })))
			if (filePath.endsWith('undefined.json')) return undefined
			if (filePath.endsWith('null.json')) return null
		},
		async buildEnd() {
			await compileFiles(['BP/generated/late.json'])
			await outputFileSystem.writeJson('stamp-report.json', { buildStarts, handles: sameHandle })
		},
	}
}
