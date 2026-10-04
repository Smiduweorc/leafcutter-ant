// The JavaScript half of Dash's plugin host, evaluated once per engine
// after runtime.js, as a function of `host` (the Rust functions it calls),
// what runtime.js returns, and pathe 2.0.2's dirname and join. It returns
// what the Rust plugin host (src/js/plugin.rs) calls back into.
//
// The parts marked as copied are unchanged from the published builds of
// @bridge-editor/common-utils 0.3.3 (dist/common-utils.es.js),
// @bridge-editor/dash-compiler 0.13.0 (dist/dash-compiler.es.js) and
// @bridge-editor/mc-project-core 0.5.0 (dist/mc-project-core.es.js), each
// Copyright (c) 2021 bridge-team, MIT; see NOTICE.md.
// They run here as the JavaScript they are, so that scripts and the values
// they return meet the code TS Dash runs.
(host, runtime, pathe) => {
	const { Runtime, FileSystem, console } = runtime
	const { dirname, join } = pathe
	const json5 = { parse: (text) => host.parseJson5(text) }

	// common-utils's isMatch, on the Rust port of its picomatch.
	function isMatch(path2, pattern) {
		if (Array.isArray(pattern)) return pattern.some((p) => isMatch(path2, p))
		return host.isMatch(path2, pattern)
	}

	// Copied: common-utils 0.3.3, lines 1824 to 1863, 1868 to 1884 and 2019
	// to 2106.
	function _walkObject(keys, current, onReach) {
	  var _a, _b;
	  if (current === void 0)
	    return;
	  if (keys.length === 0)
	    return onReach(current);
	  if (typeof current !== "object" || current === null)
	    return;
	  const key = keys.shift();
	  if (key === "**") {
	    while (keys.length > 0 && keys[0] === "*") {
	      keys.shift();
	    }
	    const matcher = ["**", ...keys].join("/");
	    const collectedPaths = new Set();
	    collectAllPaths("", current, collectedPaths);
	    for (const path2 of collectedPaths) {
	      if (isMatch(path2, matcher)) {
	        _walkObject(path2.split("/"), current, onReach);
	      }
	    }
	  } else if (key.startsWith("*")) {
	    let filterRegExp = void 0;
	    if (key.length >= 1)
	      filterRegExp = new RegExp((_b = (_a = key.match(/(\*{)(.+)(})/)) == null ? void 0 : _a[2]) != null ? _b : ".*");
	    for (const key2 in current) {
	      if (filterRegExp && key2.match(filterRegExp) !== null)
	        _walkObject([...keys], current[key2], onReach);
	    }
	  } else
	    _walkObject(keys, current[key], onReach);
	}
	function collectAllPaths(currPath, current, allPaths) {
	  for (const key in current) {
	    allPaths.add(`${currPath}${key}`);
	    if (typeof current[key] === "object") {
	      collectAllPaths(`${currPath}${key}/`, current[key], allPaths);
	    }
	  }
	}
	function setObjectAt(path2, obj, onSet) {
	  const keys = path2.length === 0 || path2 === "/" ? [] : path2.split("/");
	  if (keys.length === 0)
	    return;
	  const lastKey = keys.pop();
	  _walkObject(keys, obj, (currentObj) => {
	    if (typeof currentObj !== "object")
	      return;
	    if (lastKey === "*") {
	      for (const key in currentObj) {
	        currentObj[key] = onSet(currentObj[key]);
	      }
	    } else if (currentObj[lastKey] !== void 0) {
	      currentObj[lastKey] = onSet(currentObj[lastKey]);
	    }
	  });
	}
	function castType(value) {
	  if (value === "true") {
	    return true;
	  } else if (value === "false") {
	    return false;
	  } else if (value === "null") {
	    return null;
	  } else if (value === "undefined") {
	    return void 0;
	  } else if (isNumeric(value)) {
	    return Number(value);
	  } else if (typeof value === "string") {
	    if (value.startsWith('"') && value.endsWith('"'))
	      return value.slice(1, -1);
	    return value;
	  } else {
	    return value;
	  }
	}
	function isNumeric(value) {
	  return !isNaN(Number(value));
	}
	function tokenizeCommand(command) {
	  let curlyBrackets = 0;
	  let squareBrackets = 0;
	  let inQuotes = false;
	  let i = 0;
	  let wordStart = 0;
	  let word = "";
	  let tokens = [];
	  while (i < command.length) {
	    if (command[i] === "^" && word[0] === "^" || command[i] === "~" && word[0] === "~") {
	      tokens.push({
	        startColumn: wordStart,
	        endColumn: i,
	        word
	      });
	      wordStart = i + 1;
	      word = command[i];
	    } else if (command[i] === '"') {
	      word += command[i];
	      if (inQuotes) {
	        tokens.push({
	          startColumn: wordStart,
	          endColumn: i,
	          word
	        });
	        wordStart = i + 1;
	        word = "";
	      }
	      inQuotes = !inQuotes;
	    } else if (command[i] === " " || command[i] === "	") {
	      if (inQuotes) {
	        word += command[i];
	        i++;
	        continue;
	      }
	      if (curlyBrackets === 0 && squareBrackets === 0 && word !== "") {
	        tokens.push({
	          startColumn: wordStart,
	          endColumn: i,
	          word
	        });
	        wordStart = i + 1;
	        word = "";
	      }
	    } else {
	      if (command[i] === "{") {
	        curlyBrackets++;
	      } else if (command[i] === "}") {
	        curlyBrackets--;
	      } else if (command[i] === "[") {
	        squareBrackets++;
	      } else if (command[i] === "]") {
	        squareBrackets--;
	      }
	      if (command[i].trim() !== "")
	        word += command[i];
	    }
	    i++;
	  }
	  tokens.push({
	    startColumn: wordStart,
	    endColumn: i,
	    word
	  });
	  return { tokens };
	}

	// Copied: dash-compiler 0.13.0, lines 1070 to 1318 (custom commands).
	function transformCommands(commands, dependencies, includeComments, nestingDepth = 0) {
	  const processedCommands = [];
	  for (const writtenCommand of commands) {
	    if (!writtenCommand.startsWith("/")) {
	      processedCommands.push(writtenCommand);
	      continue;
	    }
	    const [commandName, ...args] = tokenizeCommand(writtenCommand.slice(1)).tokens.map(({ word }) => word);
	    const command = dependencies[`command#${commandName}`];
	    if (commandName === "execute") {
	      let nestedCommandIndex = 4;
	      if (args[nestedCommandIndex] === "detect") {
	        nestedCommandIndex += 6;
	      }
	      if (args[nestedCommandIndex] === void 0) {
	        processedCommands.push(writtenCommand);
	        continue;
	      }
	      const [nestedCommandName, ...nestedArgs] = args.slice(nestedCommandIndex);
	      const nestedCommand = dependencies[`command#${nestedCommandName}`];
	      if (!(nestedCommand instanceof Command)) {
	        processedCommands.push(writtenCommand);
	        continue;
	      }
	      processedCommands.push(...nestedCommand.process(`${nestedCommandName} ${nestedArgs.join(" ")}`, dependencies, nestingDepth + 1).map((command2) => command2.startsWith("/") ? `/execute ${args.slice(0, nestedCommandIndex).join(" ")} ${command2.slice(1)}` : command2));
	      continue;
	    } else if (!(command instanceof Command)) {
	      processedCommands.push(writtenCommand);
	      continue;
	    }
	    processedCommands.push(...command.process(writtenCommand, dependencies, nestingDepth));
	  }
	  return processedCommands.filter((command) => includeComments || !command.startsWith("#")).map((command) => command.trim());
	}
	const v1Compat = (v1CompatModule) => ({
	  register: (commandClass) => {
	    v1CompatModule.command = ({ name, schema, template }) => {
	      name(commandClass.command_name);
	      schema([]);
	      template((commandArgs) => {
	        const command = new commandClass();
	        return command.onApply(commandArgs);
	      });
	    };
	  }
	});
	class Command {
	  constructor(console2, commandSrc, mode, v1Compat2) {
	    this.console = console2;
	    this.commandSrc = commandSrc;
	    this.mode = mode;
	    this.v1Compat = v1Compat2;
	  }
	  get name() {
	    var _a;
	    return (_a = this._name) != null ? _a : "unknown";
	  }
	  async load(jsRuntime, filePath, type) {
	    const v1CompatModule = { command: null };
	    const module = await jsRuntime.run(filePath, {
	      console: this.console,
	      defineCommand: (x) => x,
	      Bridge: this.v1Compat ? v1Compat(v1CompatModule) : void 0
	    }).catch((err) => {
	      this.console.error(`Failed to execute command ${this.name}: ${err}`);
	      return null;
	    });
	    if (!module)
	      return null;
	    if (typeof module.__default__ !== "function") {
	      if (v1CompatModule.command) {
	        module.__default__ = v1CompatModule.command;
	      } else {
	        this.console.error(`Component ${filePath} is not a valid component. Expected a function as the default export.`);
	        return false;
	      }
	    }
	    const name = (name2) => this._name = name2;
	    let schema = (schema2) => this.schema = schema2;
	    let template = () => {
	    };
	    if (!type || type === "server") {
	      schema = () => {
	      };
	      template = (func) => {
	        this.template = (commandArgs, opts) => {
	          try {
	            return func(commandArgs, opts);
	          } catch (err) {
	            this.console.error(err);
	            return [];
	          }
	        };
	      };
	    }
	    await module.__default__({
	      name,
	      schema,
	      template
	    });
	  }
	  process(command, dependencies, nestingDepth) {
	    var _a;
	    if (command.startsWith("/"))
	      command = command.slice(1);
	    const [commandName, ...args] = tokenizeCommand(command).tokens.map(({ word }) => word);
	    const commands = (_a = this.template) == null ? void 0 : _a.call(this, args.map((arg) => castType(arg)), {
	      compilerMode: this.mode,
	      commandNestingDepth: nestingDepth,
	      compileCommands: (customCommands) => {
	        return transformCommands(customCommands.map((command2) => command2.startsWith("/") ? command2 : `/${command2}`), dependencies, false, nestingDepth + 1).map((command2) => command2.startsWith("/") ? command2.slice(1) : command2);
	      }
	    });
	    let processedCommands = [];
	    if (typeof commands === "string")
	      processedCommands = commands.split("\n");
	    else if (Array.isArray(commands))
	      processedCommands = commands.filter((command2) => typeof command2 === "string");
	    else {
	      const errrorMsg = `Failed to process command ${this._name}; Invalid command template return type: Expected string[] or string, received ${typeof commands}`;
	      this.console.error(errrorMsg);
	      processedCommands.push(`# ${errrorMsg}`);
	    }
	    return processedCommands.map((command2) => command2.startsWith("/") || command2.startsWith("#") ? command2 : `/${command2}`);
	  }
	  getSchema() {
	    if (!this.schema)
	      return [{ commandName: this.name }];
	    else if (Array.isArray(this.schema)) {
	      if (this.schema.length === 0)
	        return [{ commandName: this.name }];
	      return this.schema.map((schema) => ({
	        commandName: this.name,
	        ...schema
	      }));
	    }
	    if (!this.schema.commandName)
	      this.schema.commandName = this.name;
	    return [this.schema];
	  }
	  toString() {
	    return this.commandSrc;
	  }
	}
	const CustomCommandsPlugin = ({
	  projectConfig,
	  jsRuntime,
	  console: console2,
	  fileType: fileTypeLib,
	  requestJsonData,
	  options
	}) => {
	  const resolve = (packId, path) => projectConfig.resolvePackPath(packId, path);
	  const isCommand = (filePath) => filePath && fileTypeLib.getId(filePath) === "customCommand";
	  const isMcfunction = (filePath) => filePath && fileTypeLib.getId(filePath) === "function";
	  const cachedPaths = /* @__PURE__ */ new Map();
	  const loadCommandsFor = (filePath) => {
	    if (cachedPaths.has(filePath))
	      return cachedPaths.get(filePath);
	    const commandLocs = options.include[fileTypeLib.getId(filePath)];
	    cachedPaths.set(filePath, commandLocs);
	    return commandLocs;
	  };
	  const withSlashPrefix = (filePath) => {
	    var _a, _b, _c;
	    return (_c = (_b = (_a = fileTypeLib.get(filePath)) == null ? void 0 : _a.meta) == null ? void 0 : _b.commandsUseSlash) != null ? _c : false;
	  };
	  return {
	    async buildStart() {
	      options.include = Object.assign(await requestJsonData("data/packages/minecraftBedrock/location/validCommand.json"), options.include);
	      cachedPaths.clear();
	    },
	    ignore(filePath) {
	      return !isCommand(filePath) && !isMcfunction(filePath) && !loadCommandsFor(filePath);
	    },
	    transformPath(filePath) {
	      if (isCommand(filePath) && options.buildType !== "fileRequest")
	        return null;
	    },
	    async read(filePath, fileHandle) {
	      if (!fileHandle)
	        return;
	      if (isCommand(filePath) && filePath.endsWith(".js")) {
	        const file = await fileHandle.getFile();
	        return await (file == null ? void 0 : file.text());
	      } else if (isMcfunction(filePath)) {
	        const file = await fileHandle.getFile();
	        return await (file == null ? void 0 : file.text());
	      } else if (loadCommandsFor(filePath) && fileHandle) {
	        const file = await fileHandle.getFile();
	        if (!file)
	          return;
	        try {
	          return json5.parse(await file.text());
	        } catch (err) {
	          console2.error(err);
	        }
	      }
	    },
	    async load(filePath, fileContent) {
	      var _a;
	      if (isCommand(filePath)) {
	        const command = new Command(console2, fileContent, options.mode, (_a = options.v1CompatMode) != null ? _a : false);
	        await command.load(jsRuntime, filePath);
	        return command;
	      }
	    },
	    async registerAliases(filePath, fileContent) {
	      if (isCommand(filePath))
	        return [`command#${fileContent.name}`];
	    },
	    async require(filePath) {
	      if (loadCommandsFor(filePath) || isMcfunction(filePath)) {
	        return [
	          resolve("behaviorPack", "commands/**/*.[jt]s"),
	          resolve("behaviorPack", "commands/*.[jt]s")
	        ];
	      }
	    },
	    async transform(filePath, fileContent, dependencies = {}) {
	      const includePaths = loadCommandsFor(filePath);
	      if (includePaths && includePaths.length > 0) {
	        const hasSlashPrefix = withSlashPrefix(filePath);
	        includePaths.forEach((includePath) => setObjectAt(includePath, fileContent, (commands) => {
	          if (!commands)
	            return commands;
	          commands = Array.isArray(commands) ? commands : [commands];
	          const filteredCommands = [];
	          for (const command of commands) {
	            if (typeof command === "string") {
	              filteredCommands.push(command);
	              continue;
	            }
	            console2.error(`The file "${filePath}" contains invalid commands. Expected type "string" within array but got type "${typeof command}"`);
	          }
	          return transformCommands(filteredCommands.map((command) => !hasSlashPrefix && !command.startsWith("/") ? `/${command}` : command), dependencies, false).map((command) => hasSlashPrefix ? command : command.slice(1));
	        }));
	      } else if (isMcfunction(filePath)) {
	        const commands = fileContent.split("\n").map((command) => command.trim()).filter((command) => command !== "" && !command.startsWith("#")).map((command) => `/${command}`);
	        return transformCommands(commands, dependencies, true).map((command) => command.startsWith("/") ? command.slice(1) : command).join("\n");
	      }
	    },
	    finalizeBuild(filePath, fileContent) {
	      if (isCommand(filePath) && fileContent) {
	        return fileContent.toString();
	      }
	    }
	  };
	};

	// Copied: dash-compiler 0.13.0, lines 1518 to 1698 (generator scripts),
	// with the two modules the plugin registers, as Vite's `?raw` imports put
	// their TypeScript source in the bundle.
	class Collection {
	  constructor(console2) {
	    this.console = console2;
	    this.__isCollection = true;
	    this.files = /* @__PURE__ */ new Map();
	  }
	  get hasFiles() {
	    return this.files.size > 0;
	  }
	  getAll() {
	    return [...this.files.entries()];
	  }
	  get(filePath) {
	    return this.files.get(filePath);
	  }
	  clear() {
	    this.files.clear();
	  }
	  add(filePath, fileContent) {
	    if (this.files.has(filePath)) {
	      this.console.warn(`Omitting file "${filePath}" from collection because it would overwrite a previously generated file!`);
	      return;
	    }
	    this.files.set(filePath, fileContent);
	  }
	  has(filePath) {
	    return this.files.has(filePath);
	  }
	  addFrom(collection, baseDir) {
	    for (const [filePath, fileContent] of collection.getAll()) {
	      const resolvedPath = baseDir ? join(baseDir, filePath) : filePath;
	      this.add(resolvedPath, fileContent);
	    }
	  }
	}
	var GeneratorScriptModule = "import { dirname, join } from 'pathe'\r\nimport type { FileSystem } from '../../../FileSystem/FileSystem'\r\nimport type { Console } from '../../../Common/Console'\r\n// @ts-expect-error\r\nimport { Collection } from '@bridge-interal/collection'\r\n\r\ndeclare const __fileSystem: FileSystem\r\ndeclare const console: Console\r\ndeclare const __omitUsedTemplates: Set<string>\r\ndeclare const __baseDirectory: string\r\n\r\nexport interface IModuleOpts {\r\n	generatorPath: string\r\n	omitUsedTemplates: Set<string>\r\n	fileSystem: FileSystem\r\n	console: Console\r\n}\r\n\r\ninterface IUseTemplateOptions {\r\n	omitTemplate?: boolean\r\n}\r\n\r\nexport function useTemplate(filePath: string, { omitTemplate = true }: IUseTemplateOptions = {}) {\r\n	const templatePath = join(__baseDirectory, filePath)\r\n	if (omitTemplate) __omitUsedTemplates.add(templatePath)\r\n\r\n	// TODO(@solvedDev): Pipe file through compileFile API\r\n	if (filePath.endsWith('.json')) return __fileSystem.readJson(templatePath)\r\n	else return __fileSystem.readFile(templatePath).then(file => file.text())\r\n}\r\n\r\nexport function createCollection() {\r\n	return new Collection(console)\r\n}\r\n";
	var CollectionModule = "import { join } from 'pathe'\r\nimport { Console } from '../../../Common/Console'\r\n\r\nexport class Collection {\r\n	public readonly __isCollection = true\r\n	protected files = new Map<string, any>()\r\n	constructor(protected console: Console) {}\r\n\r\n	get hasFiles() {\r\n		return this.files.size > 0\r\n	}\r\n\r\n	getAll() {\r\n		return [...this.files.entries()]\r\n	}\r\n\r\n	get(filePath: string) {\r\n		return this.files.get(filePath)\r\n	}\r\n\r\n	clear() {\r\n		this.files.clear()\r\n	}\r\n	add(filePath: string, fileContent: any) {\r\n		if (this.files.has(filePath)) {\r\n			this.console.warn(`Omitting file \"${filePath}\" from collection because it would overwrite a previously generated file!`)\r\n			return\r\n		}\r\n		this.files.set(filePath, fileContent)\r\n	}\r\n	has(filePath: string) {\r\n		return this.files.has(filePath)\r\n	}\r\n	addFrom(collection: Collection, baseDir?: string) {\r\n		for (const [filePath, fileContent] of collection.getAll()) {\r\n			const resolvedPath = baseDir ? join(baseDir, filePath) : filePath\r\n			this.add(resolvedPath, fileContent)\r\n		}\r\n	}\r\n}\r\n";
	const GeneratorScriptsPlugin = ({ options, fileType, console: console2, jsRuntime, fileSystem, compileFiles, getFileMetadata, unlinkOutputFiles, addFileDependencies }) => {
	  var _a;
	  const ignoredFileTypes = /* @__PURE__ */ new Set(["gameTest", "customCommand", "customComponent", "molangAstScript", ...(_a = options.ignoredFileTypes) != null ? _a : []]);
	  const getFileType = (filePath) => fileType.getId(filePath);
	  const getFileContentType = (filePath) => {
	    var _a2;
	    const def = fileType.get(filePath, void 0, false);
	    if (!def)
	      return "raw";
	    return (_a2 = def.type) != null ? _a2 : "json";
	  };
	  const isGeneratorScript = (filePath) => !ignoredFileTypes.has(getFileType(filePath)) && (filePath.endsWith(".js") || filePath.endsWith(".ts"));
	  const getScriptExtension = (filePath) => {
	    var _a2, _b, _c, _d;
	    const fileContentType = getFileContentType(filePath);
	    if (fileContentType === "json")
	      return ".json";
	    return (_d = (_c = (_b = (_a2 = fileType.get(filePath, void 0, false)) == null ? void 0 : _a2.detect) == null ? void 0 : _b.fileExtensions) == null ? void 0 : _c[0]) != null ? _d : ".txt";
	  };
	  const transformPath = (filePath) => filePath.replace(/\.(js|ts)$/, getScriptExtension(filePath));
	  const omitUsedTemplates = /* @__PURE__ */ new Set();
	  const fileCollection = new Collection(console2);
	  const filesToUpdate = /* @__PURE__ */ new Set();
	  const usedTemplateMap = /* @__PURE__ */ new Map();
	  return {
	    buildStart() {
	      fileCollection.clear();
	      omitUsedTemplates.clear();
	      filesToUpdate.clear();
	      usedTemplateMap.clear();
	      jsRuntime.registerModule("@bridge-interal/collection", CollectionModule);
	      jsRuntime.registerModule("@bridge/generate", GeneratorScriptModule);
	      jsRuntime.registerModule("path-browserify", {
	        dirname,
	        join
	      });
	      jsRuntime.registerModule("pathe", {
	        dirname,
	        join
	      });
	    },
	    ignore(filePath) {
	      return !isGeneratorScript(filePath) && !omitUsedTemplates.has(filePath) && !fileCollection.has(filePath);
	    },
	    transformPath(filePath) {
	      if (filePath && isGeneratorScript(filePath))
	        return transformPath(filePath);
	    },
	    async read(filePath, fileHandle) {
	      if (isGeneratorScript(filePath) && fileHandle) {
	        const file = await fileHandle.getFile();
	        if (!file)
	          return;
	        return file.text();
	      }
	      const fromCollection = fileCollection.get(filePath);
	      if (fromCollection)
	        return fromCollection;
	    },
	    async load(filePath, fileContent) {
	      var _a2, _b;
	      if (!isGeneratorScript(filePath))
	        return;
	      if (!fileContent)
	        return null;
	      const currentTemplates = /* @__PURE__ */ new Set();
	      const module = await jsRuntime.run(filePath, {
	        console: console2,
	        __baseDirectory: dirname(filePath),
	        __omitUsedTemplates: omitUsedTemplates,
	        __fileSystem: fileSystem
	      }, fileContent).catch((err) => {
	        console2.error(`Failed to execute generator script "${filePath}": ${err}`);
	        return null;
	      });
	      if (!module)
	        return null;
	      if (!module.__default__) {
	        console2.error(`Expected generator script "${filePath}" to provide file content as default export!`);
	        return null;
	      }
	      const fileMetadata = getFileMetadata(filePath);
	      const previouslyUnlinkedFiles = ((_a2 = fileMetadata.get("unlinkedFiles")) != null ? _a2 : []).filter((filePath2) => !currentTemplates.has(filePath2));
	      previouslyUnlinkedFiles.forEach((file) => filesToUpdate.add(file));
	      fileMetadata.set("unlinkedFiles", [...currentTemplates]);
	      const generatedFiles = (_b = fileMetadata.get("generatedFiles")) != null ? _b : [];
	      await unlinkOutputFiles([...generatedFiles, ...currentTemplates]).catch(() => {
	      });
	      usedTemplateMap.set(filePath, currentTemplates);
	      return module.__default__;
	    },
	    require(filePath) {
	      const usedTemplates = usedTemplateMap.get(filePath);
	      if (usedTemplates)
	        return [...usedTemplates];
	    },
	    finalizeBuild(filePath, fileContent) {
	      if (fileCollection.get(filePath)) {
	        if (filePath.endsWith(".json") && typeof fileContent !== "string")
	          return JSON.stringify(fileContent, null, "	");
	        return fileContent;
	      }
	      if (omitUsedTemplates.has(filePath))
	        return null;
	      if (isGeneratorScript(filePath)) {
	        if (fileContent === null)
	          return null;
	        const fileMetadata = getFileMetadata(filePath);
	        if (fileContent.__isCollection) {
	          fileCollection.addFrom(fileContent, dirname(filePath));
	          fileMetadata.set("generatedFiles", fileContent.getAll().map(([filePath2]) => filePath2));
	          return null;
	        }
	        fileMetadata.set("generatedFiles", [transformPath(filePath)]);
	        return typeof fileContent === "object" ? JSON.stringify(fileContent) : fileContent;
	      }
	    },
	    async buildEnd() {
	      jsRuntime.deleteModule("@bridge/generate");
	      jsRuntime.deleteModule("@bridge-interal/collection");
	      jsRuntime.deleteModule("path-browserify");
	      if (filesToUpdate.size > 0)
	        await compileFiles([...filesToUpdate].filter((filePath) => !fileCollection.has(filePath)), false);
	      if (fileCollection.hasFiles)
	        await compileFiles(fileCollection.getAll().map(([filePath]) => filePath));
	    },
	    async beforeFileUnlinked(filePath) {
	      var _a2, _b;
	      if (isGeneratorScript(filePath)) {
	        let fileMetadata = null;
	        try {
	          fileMetadata = getFileMetadata(filePath);
	        } catch {
	        }
	        if (!fileMetadata)
	          return;
	        const unlinkedFiles = (_a2 = fileMetadata.get("unlinkedFiles")) != null ? _a2 : [];
	        const generatedFiles = (_b = fileMetadata.get("generatedFiles")) != null ? _b : [];
	        await unlinkOutputFiles(generatedFiles);
	        await compileFiles(unlinkedFiles);
	      }
	    }
	  };
	};

	// Copied: dash-compiler 0.13.0, lines 1699 to 1729. Its console is the
	// global one, which is the host's console here.
	function jsonStringifyWithFloatFix(json, matches, spacing = "	") {
	  let traversedKeys = [];
	  let traversedObjects = [];
	  return JSON.stringify(json, function(key, value) {
	    if (key !== "") {
	      traversedKeys.push(key);
	      traversedObjects.push(this);
	    }
	    if (typeof value !== "object") {
	      const path = traversedKeys.join("/");
	      const matcherTraversedObjects = [...traversedObjects];
	      console.log(path);
	      traversedKeys.pop();
	      traversedObjects.pop();
	      if (typeof value === "number") {
	        for (const matcher of matches) {
	          console.log(path, matcher.pathGlob, isMatch(path, matcher.pathGlob));
	          if (isMatch(path, matcher.pathGlob) && (!matcher.apply || matcher.apply(path, matcherTraversedObjects))) {
	            let result = value.toString();
	            return `$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___${result.includes(".") ? result : result + ".0"}`;
	          }
	        }
	      }
	      return value;
	    } else {
	      return value;
	    }
	  }, spacing).replaceAll(/"\$___dash___floatPropertyTruncationFix___THIS IS AUTO GENERATED AND I HATE IT___([0-9]|\.|-)+"/g, (value) => {
	    return value.substring(80, value.length - 1);
	  });
	}

	// What Dash hands every plugin (AllPlugins.ts getPluginContext), on the
	// compiler Rust runs.
	const fileSystem = new FileSystem(0)
	const outputFileSystem = host.separateOutput ? new FileSystem(1) : fileSystem
	const readFile = (filePath) => fileSystem.readFile(filePath)

	// `dash.jsRuntime` (src/Common/JsRuntime.ts), which the built-in plugins
	// run scripts in, and the runtime AllPlugins evaluates extension modules
	// in. The molang modules arrive with the molang port.
	const jsRuntime = new Runtime([["@bridge/compiler", { mode: host.mode }]], readFile)
	const pluginRuntime = new Runtime(undefined, readFile)

	// pathe 1.1.2's join, which mc-project-core resolves pack paths with,
	// normalizing in Rust.
	const join1 = (...arguments_) => {
		if (arguments_.length === 0) return "."
		let joined
		for (const argument of arguments_) {
			if (argument && argument.length > 0) {
				if (joined === void 0) joined = argument
				else joined += `/${argument}`
			}
		}
		if (joined === void 0) return "."
		return host.normalize(joined.replace(/\/\/+/g, "/"))
	}

	// Copied: mc-project-core 0.5.0's ProjectConfig (lines 4 to 58), with
	// pathe 1.1.2's join, on the config Rust read.
	const defaultPackPaths = {
		behaviorPack: "./BP",
		resourcePack: "./RP",
		skinPack: "./SP",
		worldTemplate: "./WT",
	}
	class ProjectConfig {
		constructor(basePath) {
			this.basePath = basePath
			this.data = {}
		}
		get() {
			return this.data
		}
		getRelativePackRoot(packId) {
			var _a, _b
			return (_b = (_a = this.data.packs) == null ? void 0 : _a[packId]) != null ? _b : defaultPackPaths[packId]
		}
		getAbsolutePackRoot(packId) {
			return this.resolvePackPath(packId)
		}
		resolvePackPath(packId, filePath) {
			if (!filePath && !packId) return this.basePath
			else if (!packId && filePath) return join1(this.basePath, filePath)
			else if (!filePath && packId) return join1(this.basePath, this.getRelativePackRoot(packId))
			return join1(this.basePath, `${this.getRelativePackRoot(packId)}/${filePath}`)
		}
		getAvailablePackPaths() {
			var _a
			const paths = []
			for (const packId of Object.keys((_a = this.data.packs) != null ? _a : {})) {
				paths.push(this.resolvePackPath(packId))
			}
			return paths
		}
		getAvailablePacks() {
			var _a
			const paths = {}
			for (const packId in (_a = this.data.packs) != null ? _a : {}) {
				paths[packId] = this.resolvePackPath(packId)
			}
			return paths
		}
	}
	const projectConfig = new ProjectConfig(host.projectRoot)

	// Copied: mc-project-core's PackType (lines 63 to 111), on the
	// definitions the host gave.
	class PackType {
		constructor(projectConfig, packTypes) {
			this.projectConfig = projectConfig
			this.packTypes = packTypes
			this.extensionPackTypes = new Set()
		}
		get all() {
			return this.packTypes.concat(...Array.from(this.extensionPackTypes.values()))
		}
		getFromId(packId) {
			return this.all.find((packType) => packType.id === packId)
		}
		get(filePath, tryAllPacks = false) {
			var _a
			let packTypes = (_a = this.projectConfig) == null ? void 0 : _a.getAvailablePacks()
			if (!packTypes || tryAllPacks)
				packTypes = Object.fromEntries(
					Object.keys(defaultPackPaths)
						.map((packId) => {
							var _a2
							const packPath = (_a2 = this.projectConfig) == null ? void 0 : _a2.resolvePackPath(packId)
							if (!packPath) return null
							return [packId, packPath]
						})
						.filter((pack) => pack !== null)
				)
			for (const packId in packTypes) {
				if (filePath.startsWith(packTypes[packId])) {
					return this.getFromId(packId)
				}
			}
		}
		getId(filePath) {
			var _a, _b
			return (_b = (_a = this.get(filePath)) == null ? void 0 : _a.id) != null ? _b : "unknown"
		}
		addExtensionPackType(packType) {
			this.extensionPackTypes.add(packType)
			return {
				dispose: () => this.extensionPackTypes.delete(packType),
			}
		}
	}
	const packType = new PackType(projectConfig, host.packDefinitions())

	// mc-project-core's FileType with the Deno CLI's per-path cache, as one
	// detection Rust runs for the built-ins and scripts alike. `get` returns
	// the definition objects themselves, as TS Dash does.
	class FileType {
		constructor(fileTypes) {
			this.fileTypes = fileTypes
		}
		get all() {
			return this.fileTypes
		}
		get(filePath, searchFileType, checkFileExtension = true) {
			const [kind, found] = host.findFileType(filePath, searchFileType, !!checkFileExtension)
			if (kind === "found") return this.fileTypes[found]
			if (kind === "noDetect") {
				console.log(this.fileTypes[found])
				throw new Error(`Invalid file definition, no "detect" properties`)
			}
		}
		getIds() {
			const ids = []
			for (const fileType of this.all) {
				ids.push(fileType.id)
			}
			return ids
		}
		getId(filePath) {
			var _a, _b
			return (_b = (_a = this.get(filePath)) == null ? void 0 : _a.id) != null ? _b : "unknown"
		}
		isJsonFile(filePath) {
			var _a, _b
			const language = (_b = (_a = this.get(filePath)) == null ? void 0 : _a.meta) == null ? void 0 : _b.language
			return language ? language === "json" : filePath.endsWith(".json")
		}
	}
	const fileType = new FileType(host.fileDefinitions())

	const getPluginContext = (pluginId, pluginOpts = {}) => ({
		options: {
			get mode() {
				return host.mode
			},
			get buildType() {
				return host.buildType()
			},
			...pluginOpts,
		},
		jsRuntime,
		console,
		fileSystem,
		outputFileSystem,
		projectConfig,
		projectRoot: host.projectRoot,
		packType,
		fileType,
		targetVersion: projectConfig.get().targetVersion,
		requestJsonData: (dataPath) => host.requestJsonData(dataPath),
		getAliases: (filePath) => host.getAliases(filePath),
		getAliasesWhere: (criteria) => host.allAliases().filter(criteria),
		getFileMetadata: (filePath) => {
			const file = host.findFile(filePath)
			if (file < 0) throw new Error(`File ${filePath} to get metadata from not found`)
			return {
				get(key) {
					return host.getMetadata(file, key)
				},
				set(key, value) {
					host.setMetadata(file, key, value)
				},
				delete(key) {
					host.deleteMetadata(file, key)
				},
			}
		},
		addFileDependencies: (filePath, filePaths, clearPrevious = false) => {
			const file = host.findFile(filePath)
			if (file < 0) throw new Error(`File ${filePath} to add dependency to not found`)
			if (clearPrevious) host.setRequiredFiles(file, [...new Set(filePaths)])
			else filePaths.forEach((filePath) => host.addRequiredFile(file, filePath))
		},
		getOutputPath: (filePath) => host.getOutputPath(filePath),
		unlinkOutputFiles: (filePaths) => host.unlinkOutputFiles(filePaths),
		hasComMojangDirectory: fileSystem !== outputFileSystem,
		compileFiles: (filePaths, virtual = true) => host.compileFiles(filePaths, !!virtual),
		jsonStringifyWithFloatFix,
	})

	// AllPlugins.ts loadPlugins: the plugin object of the extensions, which
	// the plugin list is looked up in (inherited properties included).
	let extensionPlugins = {}
	const setExtensions = (entries) => {
		extensionPlugins = {}
		for (const [pluginId, path] of entries) extensionPlugins[pluginId] = path
	}
	const isExtension = (pluginId) => !!extensionPlugins[pluginId]
	const evaluatePlugin = (pluginId) =>
		pluginRuntime
			.run(extensionPlugins[pluginId], { console })
			.catch((err) => {
				console.error(`Failed to execute plugin ${pluginId}: ${err}`)
				return null
			})
			.then((module) => {
				if (!module) return null
				if (typeof module.__default__ === "function") return module.__default__
				console.error(`Plugin ${pluginId} is invalid: It does not provide a function as a default export.`)
				return null
			})

	const availableHooks = [
		"buildStart",
		"buildEnd",
		"include",
		"ignore",
		"transformPath",
		"read",
		"load",
		"registerAliases",
		"require",
		"transform",
		"finalizeBuild",
		"beforeFileUnlinked",
	]
	// AllPlugins.ts addPlugin: the plugin a factory makes, and the hooks it
	// implements (Plugin.ts implementsHook).
	const createPlugin = async (pluginId, factory, pluginOpts) => {
		const plugin = await factory(getPluginContext(pluginId, pluginOpts))
		return [plugin, availableHooks.filter((hook) => typeof plugin[hook] === "function")]
	}
	// Plugin.ts: `this.plugin[hook]?.(...args)`.
	const callHook = (plugin, hook, args) => plugin[hook]?.(...args)

	// DashFile.ts setDefaultFileHandle: every `getFile()` of a file returns
	// the one read Dash started for it, which settles to null when the read
	// failed.
	const fileHandle = (filePath, bytes) => {
		const file = Promise.resolve(bytes === null ? null : new runtime.File([bytes], runtime.basename(filePath)))
		return { getFile: () => file }
	}

	return {
		jsRuntime,
		pluginRuntime,
		projectConfig,
		setProjectData: (data) => {
			projectConfig.data = data
		},
		setExtensions,
		isExtension,
		evaluatePlugin,
		createPlugin,
		callHook,
		fileHandle,
		dependencies: (entries) => Object.fromEntries(entries),
		builtIn: { CustomCommandsPlugin, GeneratorScriptsPlugin },
	}
}
