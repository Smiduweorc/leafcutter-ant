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
(host, runtime, pathe, molang) => {
	const { Runtime, FileSystem, console } = runtime
	const { CustomMolang, Molang, expressions } = molang
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

	// Copied: common-utils 0.3.3, lines 1894 to 1915 (deepMerge and
	// deepMergeAll), 1922 to 1932 (get), 1933 to 1935 (toHexString) and
	// 1940 to 1946 (hashString). `hash` is crypto.subtle's SHA-1 of the
	// UTF-8 bytes, which Rust computes from the string itself, so
	// textEncoder hands the string on in an object, which is truthy where
	// TextEncoder's empty Uint8Array is.
	function deepMerge(obj1, obj2) {
	  if (Array.isArray(obj1) && Array.isArray(obj2))
	    return obj1.concat(obj2);
	  else if (Array.isArray(obj1))
	    return obj1.concat([obj2]);
	  else if (Array.isArray(obj2))
	    return obj2.concat([obj1]);
	  else if (typeof obj2 !== "object")
	    return obj2;
	  let res = {};
	  for (const key in obj1) {
	    if (obj2[key] === void 0)
	      res[key] = obj1[key];
	    else
	      res[key] = deepMerge(obj1[key], obj2[key]);
	  }
	  for (const key in obj2) {
	    if (obj1[key] === void 0)
	      res[key] = obj2[key];
	  }
	  return res;
	}
	function get(obj, path2, defaultValue) {
	  if (path2.length === 0)
	    return obj;
	  let current = obj;
	  for (let i = 0; i < path2.length; i++) {
	    if (current === void 0)
	      return defaultValue;
	    current = current[path2[i]];
	  }
	  return current;
	}
	function toHexString(data) {
	  return Array.from(data).map((b) => ("00" + b.toString(16)).slice(-2)).join("");
	}
	async function hash(data) {
	  return host.sha1(data.str);
	}
	const textEncoder = { encode: (str) => ({ str }) };
	async function hashString(str) {
	  const rawData = textEncoder.encode(str);
	  if (!rawData)
	    return "";
	  const hashedData = await hash(rawData);
	  return toHexString(hashedData);
	}

	// Copied: dash-compiler 0.13.0, lines 177 to 294 (moLang).
	const MolangPlugin = async ({
	  fileType,
	  projectConfig,
	  requestJsonData,
	  options,
	  console: console2,
	  jsRuntime
	}) => {
	  const resolve = (packId, path) => projectConfig.resolvePackPath(packId, path);
	  const customMolang = new CustomMolang({});
	  const molangDirPaths = [
	    projectConfig.resolvePackPath("behaviorPack", "molang"),
	    projectConfig.resolvePackPath("resourcePack", "molang")
	  ];
	  const isMolangFile = (filePath) => filePath && molangDirPaths.some((path) => filePath.startsWith(`${path}/`)) && filePath.endsWith(".molang");
	  const molangScriptPath = projectConfig.resolvePackPath("behaviorPack", "scripts/molang");
	  const isMolangScript = (filePath) => filePath == null ? void 0 : filePath.startsWith(`${molangScriptPath}/`);
	  const cachedPaths = /* @__PURE__ */ new Map();
	  const loadMolangFrom = (filePath) => {
	    if (cachedPaths.has(filePath))
	      return cachedPaths.get(filePath);
	    const molangLocs = options.include[fileType.getId(filePath)];
	    cachedPaths.set(filePath, molangLocs);
	    return molangLocs;
	  };
	  let astTransformers = [];
	  return {
	    async buildStart() {
	      options.include = Object.assign(await requestJsonData("data/packages/minecraftBedrock/location/validMolang.json"), options.include);
	      cachedPaths.clear();
	    },
	    ignore(filePath) {
	      return !isMolangFile(filePath) && !isMolangScript(filePath) && !loadMolangFrom(filePath);
	    },
	    transformPath(filePath) {
	      if (isMolangFile(filePath) || isMolangScript(filePath))
	        return null;
	    },
	    async read(filePath, fileHandle) {
	      if ((isMolangFile(filePath) || isMolangScript(filePath)) && fileHandle) {
	        const file = await fileHandle.getFile();
	        return await (file == null ? void 0 : file.text());
	      } else if (loadMolangFrom(filePath) && filePath.endsWith(".json") && fileHandle) {
	        const file = await fileHandle.getFile();
	        if (!file)
	          return;
	        try {
	          return json5.parse(await file.text());
	        } catch (err) {
	          if (options.buildType !== "fileRequest")
	            console2.error(`Error within file "${filePath}": ${err}`);
	          return {
	            __error__: `Failed to load original file: ${err}`
	          };
	        }
	      }
	    },
	    async load(filePath, fileContent) {
	      if (isMolangFile(filePath) && fileContent) {
	        customMolang.parse(fileContent);
	      } else if (isMolangScript(filePath)) {
	        const module = await jsRuntime.run(filePath, { console: console2 }, fileContent).catch((err) => {
	          console2.error(`Failed to execute Molang AST script "${filePath}": ${err}`);
	          return null;
	        });
	        if (!module)
	          return null;
	        if (typeof module.__default__ === "function")
	          astTransformers.push(module.__default__);
	      }
	    },
	    async require(filePath) {
	      if (loadMolangFrom(filePath)) {
	        return [
	          resolve("behaviorPack", "scripts/molang/**/*.[jt]s"),
	          resolve("behaviorPack", "molang/**/*.molang"),
	          resolve("resourcePack", "molang/**/*.molang")
	        ];
	      }
	    },
	    async transform(filePath, fileContent) {
	      const includePaths = loadMolangFrom(filePath);
	      if (includePaths && includePaths.length > 0) {
	        includePaths.forEach((includePath) => setObjectAt(includePath, fileContent, (molang) => {
	          if (typeof molang !== "string")
	            return molang;
	          if (molang[0] === "/" || molang[0] === "@")
	            return molang;
	          if (astTransformers.length > 0) {
	            let ast = null;
	            try {
	              ast = customMolang.parse(molang);
	            } catch (err) {
	              if (options.buildType !== "fileRequest")
	                console2.error(`Error within file "${filePath}"; script "${molang}": ${err}`);
	            }
	            if (ast) {
	              for (const transformer of astTransformers) {
	                ast = ast.walk(transformer);
	              }
	              molang = ast.toString();
	            }
	          }
	          try {
	            return customMolang.transform(molang);
	          } catch (err) {
	            if (options.buildType !== "fileRequest")
	              console2.error(`Error within file "${filePath}"; script "${molang}": ${err}`);
	            return molang;
	          }
	        }));
	      }
	    },
	    buildEnd() {
	      astTransformers = [];
	    }
	  };
	};

	// Copied: dash-compiler 0.13.0, lines 309 to 1069 (compare-versions
	// 4.1.2 as Dash bundles it, Copyright (c) 2015-2021 Ole Michelsen, MIT,
	// and the custom entity, item and block components).
	function compareVersions(v1, v2) {
	  const n1 = validateAndParse(v1);
	  const n2 = validateAndParse(v2);
	  const p1 = n1.pop();
	  const p2 = n2.pop();
	  const r = compareSegments(n1, n2);
	  if (r !== 0)
	    return r;
	  if (p1 && p2) {
	    return compareSegments(p1.split("."), p2.split("."));
	  } else if (p1 || p2) {
	    return p1 ? -1 : 1;
	  }
	  return 0;
	}
	const compare = (v1, v2, operator) => {
	  assertValidOperator(operator);
	  const res = compareVersions(v1, v2);
	  return operatorResMap[operator].includes(res);
	};
	const semver = /^[v^~<>=]*?(\d+)(?:\.([x*]|\d+)(?:\.([x*]|\d+)(?:\.([x*]|\d+))?(?:-([\da-z\-]+(?:\.[\da-z\-]+)*))?(?:\+[\da-z\-]+(?:\.[\da-z\-]+)*)?)?)?$/i;
	const validateAndParse = (v) => {
	  if (typeof v !== "string") {
	    throw new TypeError("Invalid argument expected string");
	  }
	  const match = v.match(semver);
	  if (!match) {
	    throw new Error(`Invalid argument not valid semver ('${v}' received)`);
	  }
	  match.shift();
	  return match;
	};
	const isWildcard = (s) => s === "*" || s === "x" || s === "X";
	const tryParse = (v) => {
	  const n = parseInt(v, 10);
	  return isNaN(n) ? v : n;
	};
	const forceType = (a, b) => typeof a !== typeof b ? [String(a), String(b)] : [a, b];
	const compareStrings = (a, b) => {
	  if (isWildcard(a) || isWildcard(b))
	    return 0;
	  const [ap, bp] = forceType(tryParse(a), tryParse(b));
	  if (ap > bp)
	    return 1;
	  if (ap < bp)
	    return -1;
	  return 0;
	};
	const compareSegments = (a, b) => {
	  for (let i = 0; i < Math.max(a.length, b.length); i++) {
	    const r = compareStrings(a[i] || 0, b[i] || 0);
	    if (r !== 0)
	      return r;
	  }
	  return 0;
	};
	const operatorResMap = {
	  ">": [1],
	  ">=": [0, 1],
	  "=": [0],
	  "<=": [-1, 0],
	  "<": [-1]
	};
	const allowedOperators = Object.keys(operatorResMap);
	const assertValidOperator = (op) => {
	  if (typeof op !== "string") {
	    throw new TypeError(`Invalid operator type, expected string but got ${typeof op}`);
	  }
	  if (allowedOperators.indexOf(op) === -1) {
	    throw new Error(`Invalid operator, expected one of ${allowedOperators.join("|")}`);
	  }
	};
	const v1Compat$1 = (v1CompatModule, fileType) => ({
	  register: (componentClass) => {
	    var _a;
	    if (((_a = componentClass.type) != null ? _a : "entity") !== fileType)
	      return;
	    v1CompatModule.component = ({ name, schema, template }) => {
	      const component = new componentClass();
	      name(componentClass.component_name);
	      schema(transformV1AutoCompletions(Object.values(component.onPropose())[0]));
	      template((componentArgs, { create }) => {
	        create(component.onApply(componentArgs, "components")[`minecraft:${fileType}`]);
	      });
	    };
	  }
	});
	function transformV1AutoCompletions(completions) {
	  const v2Completions = {};
	  const keys = Object.keys(completions);
	  if (keys.length === 1 && keys[0].startsWith("$dynamic.list.")) {
	    return {
	      type: "array",
	      items: transformV1AutoCompletions(Object.values(completions)[0])
	    };
	  }
	  for (const [propertyName, value] of Object.entries(completions)) {
	    if (propertyName.startsWith("$"))
	      continue;
	    if (typeof value === "string")
	      v2Completions[propertyName] = transformV1Value(value);
	    else if (Array.isArray(value))
	      v2Completions[propertyName] = { enum: value };
	    else if (value === "object")
	      v2Completions[propertyName] = transformV1AutoCompletions(value);
	  }
	  return { type: "object", properties: v2Completions };
	}
	function transformV1Value(value) {
	  switch (value) {
	    case "$general.boolean":
	      return { type: "boolean" };
	    case "$general.number":
	      return { type: "integer" };
	    case "$general.decimal":
	      return { type: "number" };
	    default:
	      return {
	        type: [
	          "number",
	          "integer",
	          "string",
	          "boolean",
	          "object",
	          "array"
	        ]
	      };
	  }
	}
	class Component {
	  constructor(console2, fileType, componentSrc, mode, v1Compat2, targetVersion) {
	    this.console = console2;
	    this.fileType = fileType;
	    this.componentSrc = componentSrc;
	    this.mode = mode;
	    this.v1Compat = v1Compat2;
	    this.targetVersion = targetVersion;
	    this.animations = [];
	    this.animationControllers = [];
	    this.createOnPlayer = [];
	    this.dialogueScenes = {};
	    this.serverFiles = [];
	    this.clientFiles = {};
	    this.lifecycleHookCount = {
	      activated: 0,
	      deactivated: 0
	    };
	  }
	  setProjectConfig(projectConfig) {
	    this.projectConfig = projectConfig;
	  }
	  get name() {
	    return this._name;
	  }
	  async load(jsRuntime, filePath, type) {
	    let v1CompatModule = { component: null };
	    const module = await jsRuntime.run(filePath, {
	      defineComponent: (x) => x,
	      console: this.console,
	      Bridge: this.v1Compat ? v1Compat$1(v1CompatModule, this.fileType) : void 0
	    }).catch((err) => {
	      this.console.error(`Failed to execute component "${filePath}": ${err}`);
	      return null;
	    });
	    if (!module)
	      return false;
	    if (typeof module.__default__ !== "function") {
	      if (v1CompatModule.component) {
	        module.__default__ = v1CompatModule.component;
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
	        this.template = (componentArgs, opts) => {
	          try {
	            func(componentArgs, opts);
	          } catch (err) {
	            this.console.log(func.toString());
	            this.console.error(err);
	          }
	        };
	      };
	    }
	    await module.__default__({
	      name,
	      schema,
	      template
	    });
	    return true;
	  }
	  reset() {
	    if (this.fileType !== "item") {
	      this.animations = [];
	      this.animationControllers = [];
	    }
	    this.clientFiles = {};
	    this.serverFiles = [];
	  }
	  getSchema() {
	    return this.schema;
	  }
	  toString() {
	    return this.componentSrc;
	  }
	  create(fileContent, template, location = `minecraft:${this.fileType}`, operation) {
	    var _a;
	    const keys = location.split("/");
	    const lastKey = keys.pop();
	    const current = this.getObjAtLocation(fileContent, [...keys]);
	    current[lastKey] = (operation ? (oldData, newData) => operation(deepMerge, oldData, newData) : deepMerge)((_a = current[lastKey]) != null ? _a : {}, template != null ? template : {});
	  }
	  getObjAtLocation(fileContent, location) {
	    let current = fileContent;
	    while (location.length > 0) {
	      const key = location.shift();
	      if (current[key] === void 0) {
	        if (current[Number(key)] !== void 0) {
	          current = current[Number(key)];
	        } else {
	          current[key] = {};
	          current = current[key];
	        }
	      } else {
	        current = current[key];
	      }
	    }
	    return current;
	  }
	  async processTemplates(fileContent, componentArgs, location) {
	    var _a, _b, _c, _d, _e, _f;
	    if (typeof this.template !== "function")
	      return;
	    const identifier = (_c = (_b = (_a = fileContent[`minecraft:${this.fileType}`]) == null ? void 0 : _a.description) == null ? void 0 : _b.identifier) != null ? _c : "bridge:no_identifier";
	    const fileName = await hashString(`${this.name}/${identifier}`);
	    const projectNamespace = (_f = (_e = (_d = this.projectConfig) == null ? void 0 : _d.get()) == null ? void 0 : _e.namespace) != null ? _f : "bridge";
	    let folderNamespace;
	    if (projectNamespace.includes("_")) {
	      const studioname = projectNamespace.split("_")[0];
	      const packname = projectNamespace.split("_").slice(1).join("_");
	      folderNamespace = `${studioname}/${packname}`;
	    } else {
	      folderNamespace = "bridge";
	    }
	    const animation = (animation2, molangCondition) => {
	      this.animations.push([animation2, molangCondition]);
	      return this.getShortAnimName("a", fileName, this.animations.length - 1);
	    };
	    const animationController = (animationController2, molangCondition) => {
	      this.animationControllers.push([animationController2, molangCondition]);
	      return this.getShortAnimName("ac", fileName, this.animationControllers.length - 1);
	    };
	    const lootTable = (lootTableDef) => {
	      const lootId = `loot_tables/${folderNamespace}/${this.getShortAnimName("lt", fileName, this.serverFiles.length)}.json`;
	      this.serverFiles.push([lootId, lootTableDef]);
	      return lootId;
	    };
	    const tradeTable = (tradeTableDef) => {
	      const tradeId = `trading/${folderNamespace}/${this.getShortAnimName("tt", fileName, this.serverFiles.length)}.json`;
	      this.serverFiles.push([tradeId, tradeTableDef]);
	      return tradeId;
	    };
	    const recipe = (recipeDef) => {
	      this.serverFiles.push([`recipes/${folderNamespace}/${this.getShortAnimName("recipe", fileName, this.serverFiles.length)}.json`, recipeDef]);
	    };
	    const spawnRule = (spawnRuleDef) => {
	      this.serverFiles.push([`spawn_rules/${folderNamespace}/${this.getShortAnimName("sr", fileName, this.serverFiles.length)}.json`, spawnRuleDef]);
	    };
	    const permutationEventName = (await hashString(`${this.name}/${location}`)).slice(0, 16);
	    const onActivated = (eventResponse) => this.registerLifecycleHook(fileContent, location, eventResponse, permutationEventName, "activated");
	    const onDeactivated = (eventResponse) => this.registerLifecycleHook(fileContent, location, eventResponse, permutationEventName, "deactivated");
	    if (this.fileType === "entity") {
	      this.template(componentArgs != null ? componentArgs : {}, {
	        mode: this.mode,
	        compilerMode: this.mode,
	        sourceEntity: () => JSON.parse(JSON.stringify(fileContent)),
	        create: (template, location2, operation) => this.create(fileContent, template, location2, operation),
	        location,
	        identifier,
	        projectNamespace,
	        animationController,
	        animation,
	        lootTable,
	        tradeTable,
	        spawnRule,
	        dialogueScene: !this.targetVersion || compare(this.targetVersion, "1.17.10", ">=") ? (scene, openDialogue = true) => {
	          if (!this.dialogueScenes[fileName])
	            this.dialogueScenes[fileName] = [];
	          this.dialogueScenes[fileName].push(scene);
	          if (scene.scene_tag && openDialogue)
	            onActivated({
	              run_command: {
	                command: [`/dialogue open @s @p ${scene.scene_tag}`]
	              }
	            });
	        } : void 0,
	        onActivated,
	        onDeactivated,
	        client: {
	          create: (clientEntity, formatVersion = "1.10.0") => {
	            this.clientFiles[`entity/${folderNamespace}/${fileName}.json`] = {
	              format_version: formatVersion,
	              "minecraft:client_entity": Object.assign({
	                description: {
	                  identifier
	                }
	              }, clientEntity)
	            };
	          }
	        }
	      });
	    } else if (this.fileType === "item") {
	      this.template(componentArgs != null ? componentArgs : {}, {
	        mode: this.mode,
	        compilerMode: this.mode,
	        sourceItem: () => JSON.parse(JSON.stringify(fileContent)),
	        create: (template, location2, operation) => this.create(fileContent, template, location2, operation),
	        location,
	        identifier,
	        projectNamespace,
	        lootTable,
	        recipe,
	        player: {
	          animationController,
	          animation,
	          create: (template, location2, operation) => {
	            this.createOnPlayer.push([location2 != null ? location2 : `minecraft:entity`, template, operation]);
	          }
	        }
	      });
	    } else if (this.fileType === "block") {
	      this.template(componentArgs != null ? componentArgs : {}, {
	        mode: this.mode,
	        compilerMode: this.mode,
	        sourceBlock: () => JSON.parse(JSON.stringify(fileContent)),
	        create: (template, location2, operation) => this.create(fileContent, template, location2, operation),
	        lootTable,
	        recipe,
	        location,
	        identifier,
	        projectNamespace
	      });
	    }
	  }
	  async processAdditionalFiles(filePath, fileContent, isPlayerFile = false) {
	    var _a, _b, _c, _d, _e, _f, _g, _h, _i;
	    const bpRoot = (_b = (_a = this.projectConfig) == null ? void 0 : _a.getRelativePackRoot("behaviorPack")) != null ? _b : "BP";
	    const rpRoot = (_c = this.projectConfig) == null ? void 0 : _c.getRelativePackRoot("resourcePack");
	    const identifier = isPlayerFile ? "minecraft:player" : (_f = (_e = (_d = fileContent[`minecraft:${this.fileType}`]) == null ? void 0 : _d.description) == null ? void 0 : _e.identifier) != null ? _f : "bridge:no_identifier";
	    const projectNamespace = (_i = (_h = (_g = this.projectConfig) == null ? void 0 : _g.get()) == null ? void 0 : _h.namespace) != null ? _i : "bridge";
	    let folderNamespace;
	    if (projectNamespace.includes("_")) {
	      const studioname = projectNamespace.split("_")[0];
	      const packname = projectNamespace.split("_").slice(1).join("_");
	      folderNamespace = `${studioname}/${packname}`;
	    } else {
	      folderNamespace = "bridge";
	    }
	    const fileName = (await hashString(`${this.name}/${identifier}`)).slice(0, 25);
	    const animFileName = `${bpRoot}/animations/${folderNamespace}/${fileName}.json`;
	    const animControllerFileName = `${bpRoot}/animation_controllers/${folderNamespace}/${fileName}.json`;
	    if (identifier === "minecraft:player") {
	      this.createOnPlayer.forEach(([location, template, operation]) => {
	        this.create(fileContent, template, location, operation);
	      });
	    }
	    if (!rpRoot) {
	      this.clientFiles = {};
	      this.console.error(`[${this.name}] Dash was unable to load the root of your resource pack and therefore cannot generate client files for this component.`);
	    }
	    let anims = {};
	    if (this.fileType !== "item" || identifier === "minecraft:player") {
	      anims = {
	        [animFileName]: {
	          baseFile: filePath,
	          fileContent: this.createAnimations(fileName, fileContent)
	        },
	        [animControllerFileName]: {
	          baseFile: filePath,
	          fileContent: this.createAnimationControllers(fileName, fileContent)
	        }
	      };
	    }
	    return {
	      ...anims,
	      [join(bpRoot, `dialogue/${folderNamespace}/${fileName}.json`)]: this.dialogueScenes[fileName] && this.dialogueScenes[fileName].length > 0 ? {
	        baseFile: filePath,
	        fileContent: JSON.stringify({
	          format_version: this.targetVersion,
	          "minecraft:npc_dialogue": {
	            scenes: this.dialogueScenes[fileName]
	          }
	        }, null, "	")
	      } : void 0,
	      ...Object.fromEntries(this.serverFiles.map(([currFilePath, fileDef]) => [
	        join(bpRoot, currFilePath),
	        {
	          baseFile: filePath,
	          fileContent: JSON.stringify(fileDef, null, "	")
	        }
	      ])),
	      ...Object.fromEntries(Object.entries(this.clientFiles).map(([currFilePath, jsonContent]) => [
	        join(rpRoot, currFilePath),
	        {
	          baseFile: filePath,
	          fileContent: JSON.stringify(jsonContent, null, "	")
	        }
	      ]))
	    };
	  }
	  createAnimations(fileName, fileContent) {
	    var _a, _b, _c;
	    if (this.animations.length === 0)
	      return;
	    const projectNamespace = (_c = (_b = (_a = this.projectConfig) == null ? void 0 : _a.get()) == null ? void 0 : _b.namespace) != null ? _c : "bridge";
	    let id = 0;
	    const animations = { format_version: "1.10.0", animations: {} };
	    for (const [anim, condition] of this.animations) {
	      if (!anim) {
	        id++;
	        continue;
	      }
	      const animId = this.getAnimName("animation", projectNamespace, fileName, id);
	      const shortAnimId = this.getShortAnimName("a", fileName, id);
	      animations.animations[animId] = anim;
	      this.create(fileContent, {
	        animations: {
	          [shortAnimId]: animId
	        }
	      }, "minecraft:entity/description");
	      if (condition !== false)
	        this.create(fileContent, {
	          scripts: {
	            animate: [!condition ? shortAnimId : { [shortAnimId]: condition }]
	          }
	        }, "minecraft:entity/description");
	      id++;
	    }
	    return JSON.stringify(animations, null, "	");
	  }
	  createAnimationControllers(fileName, fileContent) {
	    var _a, _b, _c;
	    if (this.animationControllers.length === 0)
	      return;
	    const projectNamespace = (_c = (_b = (_a = this.projectConfig) == null ? void 0 : _a.get()) == null ? void 0 : _b.namespace) != null ? _c : "bridge";
	    let id = 0;
	    const animationControllers = {
	      format_version: "1.10.0",
	      animation_controllers: {}
	    };
	    for (const [anim, condition] of this.animationControllers) {
	      if (!anim) {
	        id++;
	        continue;
	      }
	      const animId = this.getAnimName("controller.animation", projectNamespace, fileName, id);
	      const shortAnimId = this.getShortAnimName("ac", fileName, id);
	      animationControllers.animation_controllers[animId] = anim;
	      this.create(fileContent, {
	        animations: {
	          [shortAnimId]: animId
	        }
	      }, "minecraft:entity/description");
	      if (condition !== false)
	        this.create(fileContent, {
	          scripts: {
	            animate: [!condition ? shortAnimId : { [shortAnimId]: condition }]
	          }
	        }, "minecraft:entity/description");
	      id++;
	    }
	    return JSON.stringify(animationControllers, null, "	");
	  }
	  getAnimName(prefix, namespace, fileName, id) {
	    return `${prefix}.${namespace}.${fileName}_${id}`;
	  }
	  getShortAnimName(category, fileName, id) {
	    var _a;
	    return `${(_a = fileName.slice(0, 16)) != null ? _a : "bridge_auto"}_${category}_${id}`;
	  }
	  registerLifecycleHook(fileContent, location, eventResponse, permutationEventName, type) {
	    if (!fileContent[`minecraft:${this.fileType}`].events)
	      fileContent[`minecraft:${this.fileType}`].events = {};
	    const entityEvents = fileContent[`minecraft:${this.fileType}`].events;
	    if (type === "activated" && location === `minecraft:${this.fileType}/components`) {
	      if (!entityEvents["minecraft:entity_spawned"])
	        entityEvents["minecraft:entity_spawned"] = {};
	      this.addEventReponse(entityEvents["minecraft:entity_spawned"], eventResponse);
	    } else if (this.fileType === "entity" && location.startsWith(`minecraft:${this.fileType}/component_groups/`)) {
	      const componentGroupName = location.split("/").pop();
	      const eventsWithReferences = this.findComponentGroupReferences(entityEvents, type === "activated" ? "add" : "remove", componentGroupName);
	      eventsWithReferences.forEach((eventWithReference) => this.addEventReponse(eventWithReference, eventResponse));
	    } else if (location.startsWith(`minecraft:${this.fileType}/permutations/`)) {
	      const keys = location.split("/");
	      if (keys.pop() !== "components")
	        throw new Error("Invalid component location inside of permutation");
	      const permutation = this.getObjAtLocation(fileContent, [...keys]);
	      const eventName = `bridge:${permutationEventName}_${type}_${type === "activated" ? this.lifecycleHookCount.activated++ : this.lifecycleHookCount.deactivated++}`;
	      if (permutation.condition)
	        this.animationControllers.push([
	          {
	            states: {
	              default: {
	                on_entry: [`@s ${eventName}`]
	              }
	            }
	          },
	          type === "activated" ? permutation.condition : `!(${permutation.condition})`
	        ]);
	      entityEvents[eventName] = eventResponse;
	    }
	  }
	  addEventReponse(event, eventResponse) {
	    if (Array.isArray(event.sequence)) {
	      event.sequence.push(eventResponse);
	    } else if (Object.keys(event).length === 0) {
	      Object.assign(event, eventResponse);
	    } else {
	      let oldEvent = Object.assign({}, event, { filters: void 0 });
	      for (const key in event) {
	        if (key !== "filters")
	          event[key] = void 0;
	      }
	      event.sequence = [oldEvent, eventResponse];
	    }
	  }
	  findComponentGroupReferences(events, type, componentGroupName) {
	    var _a, _b;
	    let eventsWithComponentGroups = [];
	    for (const eventName in events) {
	      const event = events[eventName];
	      if (Array.isArray(event.sequence))
	        eventsWithComponentGroups.push(...this.findComponentGroupReferences(event.sequence, type, componentGroupName));
	      else if (Array.isArray(event.randomize))
	        eventsWithComponentGroups.push(...this.findComponentGroupReferences(event.randomize, type, componentGroupName));
	      else {
	        const componentGroups = (_b = (_a = event[type]) == null ? void 0 : _a.component_groups) != null ? _b : [];
	        if (componentGroups.includes(componentGroupName))
	          eventsWithComponentGroups.push(event);
	      }
	    }
	    return eventsWithComponentGroups;
	  }
	}
	function findCustomComponents(componentObjects) {
	  const components = [];
	  for (const [location, componentObject] of componentObjects) {
	    components.push(...scanComponentObject(componentObject, location));
	  }
	  return components;
	}
	function scanComponentObject(componentObject, location) {
	  const components = [];
	  for (const componentName in componentObject) {
	    if (componentName.startsWith("minecraft:") || componentName.startsWith("tag:"))
	      continue;
	    components.push([componentName, location]);
	  }
	  return components;
	}
	function createCustomComponentPlugin({ fileType, getComponentObjects }) {
	  const usedComponents = /* @__PURE__ */ new Map();
	  let createAdditionalFiles = {};
	  return ({ console: console2, projectConfig, projectRoot, compileFiles, getAliases, getAliasesWhere, options, jsRuntime, targetVersion, fileType: fileTypeLib, fileSystem }) => {
	    let playerFile = null;
	    const isPlayerFile = (filePath, getAliases2) => {
	      if (!filePath)
	        return false;
	      if (playerFile && filePath === playerFile)
	        return true;
	      const isPlayerFile2 = fileType === "item" && (fileTypeLib == null ? void 0 : fileTypeLib.getId(filePath)) === "entity" && getAliases2(filePath).includes("minecraft:player");
	      if (isPlayerFile2)
	        playerFile = filePath;
	      return isPlayerFile2;
	    };
	    const cachedIsComponent = /* @__PURE__ */ new Map();
	    const isComponent = (filePath) => {
	      if (!filePath)
	        return false;
	      if (cachedIsComponent.has(filePath))
	        return cachedIsComponent.get(filePath);
	      const isComponent2 = options.v1CompatMode ? filePath.includes(`/components/`) : (fileTypeLib == null ? void 0 : fileTypeLib.getId(filePath)) === `customComponent` && filePath.includes(`/${fileType}/`);
	      cachedIsComponent.set(filePath, isComponent2);
	      return isComponent2;
	    };
	    const cachedMayUseComponents = /* @__PURE__ */ new Map();
	    const mayUseComponent = (filePath) => {
	      if (!filePath)
	        return false;
	      if (cachedMayUseComponents.has(filePath))
	        return cachedMayUseComponents.get(filePath);
	      const result = (fileTypeLib == null ? void 0 : fileTypeLib.getId(filePath)) === fileType;
	      cachedMayUseComponents.set(filePath, result);
	      return result;
	    };
	    let hasComponentFiles = false;
	    return {
	      async buildStart() {
	        var _a, _b;
	        usedComponents.clear();
	        cachedIsComponent.clear();
	        cachedMayUseComponents.clear();
	        playerFile = null;
	        createAdditionalFiles = {};
	        hasComponentFiles = (await fileSystem.allFiles(`${projectRoot}${(_b = (_a = projectConfig.get().packs) == null ? void 0 : _a.behaviorPack) == null ? void 0 : _b.substring(1)}/components`).catch(() => [])).length > 0;
	      },
	      ignore(filePath) {
	        return !createAdditionalFiles[filePath] && !isComponent(filePath) && !mayUseComponent(filePath) && !(fileType === "item" && fileTypeLib.getId(filePath) === "entity");
	      },
	      transformPath(filePath) {
	        if (isComponent(filePath) && options.buildType !== "fileRequest")
	          return null;
	      },
	      async read(filePath, fileHandle) {
	        if (!fileHandle)
	          return createAdditionalFiles[filePath] ? json5.parse(createAdditionalFiles[filePath].fileContent) : void 0;
	        if (isComponent(filePath)) {
	          hasComponentFiles = true;
	          const file = await fileHandle.getFile();
	          return await (file == null ? void 0 : file.text());
	        } else if (mayUseComponent(filePath) || isPlayerFile(filePath, getAliases)) {
	          const file = await fileHandle.getFile();
	          if (!file)
	            return;
	          try {
	            return json5.parse(await file.text());
	          } catch (err) {
	            if (options.buildType !== "fileRequest")
	              console2.error(`Error within file "${filePath}": ${err}`);
	            return {
	              __error__: `Failed to load original file: ${err}`
	            };
	          }
	        }
	      },
	      async load(filePath, fileContent) {
	        if (!hasComponentFiles)
	          return;
	        if (isComponent(filePath) && typeof fileContent === "string") {
	          const component = new Component(console2, fileType, fileContent, options.mode, !!options.v1CompatMode, targetVersion);
	          component.setProjectConfig(projectConfig);
	          const loadedCorrectly = await component.load(jsRuntime, filePath);
	          return loadedCorrectly ? component : fileContent;
	        }
	      },
	      async registerAliases(filePath, fileContent) {
	        if (!hasComponentFiles)
	          return;
	        if (isComponent(filePath)) {
	          return [`${fileType}Component#${fileContent.name}`];
	        }
	      },
	      async require(filePath, fileContent) {
	        if (!hasComponentFiles)
	          return;
	        if (isPlayerFile(filePath, getAliases)) {
	          return getAliasesWhere((alias) => alias.startsWith("itemComponent#"));
	        }
	        if (mayUseComponent(filePath)) {
	          const components = findCustomComponents(getComponentObjects(fileContent));
	          usedComponents.set(filePath, components);
	          return components.map((component) => `${fileType}Component#${component[0]}`);
	        } else if (createAdditionalFiles[filePath]) {
	          return [createAdditionalFiles[filePath].baseFile];
	        }
	      },
	      async transform(filePath, fileContent, dependencies = {}) {
	        var _a;
	        if (!hasComponentFiles)
	          return;
	        if (isPlayerFile(filePath, getAliases)) {
	          const itemComponents = Object.entries(dependencies).filter(([depName]) => depName.startsWith("itemComponent#")).map(([_, component]) => component);
	          for (const component of itemComponents) {
	            if (!component)
	              return;
	            createAdditionalFiles = deepMerge(createAdditionalFiles, await component.processAdditionalFiles(filePath, fileContent, true));
	          }
	        } else if (mayUseComponent(filePath)) {
	          const components = /* @__PURE__ */ new Set();
	          for (const [componentName, location] of (_a = usedComponents.get(filePath)) != null ? _a : []) {
	            const component = dependencies[`${fileType}Component#${componentName}`];
	            if (!component)
	              continue;
	            const parentObj = get(fileContent, location.split("/"), {});
	            const componentArgs = parentObj[componentName];
	            delete parentObj[componentName];
	            await component.processTemplates(fileContent, componentArgs, location);
	            components.add(component);
	          }
	          for (const component of components) {
	            createAdditionalFiles = deepMerge(createAdditionalFiles, await component.processAdditionalFiles(filePath, fileContent));
	          }
	          for (const component of components) {
	            component.reset();
	          }
	        }
	      },
	      finalizeBuild(filePath, fileContent) {
	        if (!hasComponentFiles)
	          return;
	        if (isComponent(filePath) && fileContent) {
	          return fileContent.toString();
	        } else if (mayUseComponent(filePath) || createAdditionalFiles[filePath])
	          return JSON.stringify(fileContent, null, "	");
	      },
	      async buildEnd() {
	        if (!hasComponentFiles)
	          return;
	        if (options.buildType === "fileRequest")
	          return;
	        createAdditionalFiles = Object.fromEntries(Object.entries(createAdditionalFiles).filter(([_, fileData]) => (fileData == null ? void 0 : fileData.fileContent) !== void 0).map(([filePath, fileData]) => [join(projectRoot, filePath), fileData]));
	        const compilePaths = Object.keys(createAdditionalFiles);
	        if (compilePaths.length > 0)
	          await compileFiles(compilePaths);
	        createAdditionalFiles = {};
	      }
	    };
	  };
	}
	const CustomEntityComponentPlugin = createCustomComponentPlugin({
	  fileType: "entity",
	  getComponentObjects: (fileContent) => {
	    var _a, _b, _c, _d, _e, _f;
	    return [
	      ["minecraft:entity/components", (_b = (_a = fileContent == null ? void 0 : fileContent["minecraft:entity"]) == null ? void 0 : _a.components) != null ? _b : {}],
	      ...Object.entries((_d = (_c = fileContent == null ? void 0 : fileContent["minecraft:entity"]) == null ? void 0 : _c.component_groups) != null ? _d : {}).map(([groupName, groupContent]) => [`minecraft:entity/component_groups/${groupName}`, groupContent]),
	      ...((_f = (_e = fileContent == null ? void 0 : fileContent["minecraft:entity"]) == null ? void 0 : _e.permutations) != null ? _f : []).map((permutation, index) => {
	        var _a2;
	        return [`minecraft:entity/permutations/${index}/components`, (_a2 = permutation == null ? void 0 : permutation.components) != null ? _a2 : {}];
	      })
	    ];
	  }
	});
	const CustomItemComponentPlugin = createCustomComponentPlugin({
	  fileType: "item",
	  getComponentObjects: (fileContent) => {
	    var _a, _b;
	    return [["minecraft:item/components", (_b = (_a = fileContent == null ? void 0 : fileContent["minecraft:item"]) == null ? void 0 : _a.components) != null ? _b : {}]];
	  }
	});
	const CustomBlockComponentPlugin = createCustomComponentPlugin({
	  fileType: "block",
	  getComponentObjects: (fileContent) => {
	    var _a, _b, _c, _d;
	    return [
	      ["minecraft:block/components", (_b = (_a = fileContent == null ? void 0 : fileContent["minecraft:block"]) == null ? void 0 : _a.components) != null ? _b : {}],
	      ...((_d = (_c = fileContent == null ? void 0 : fileContent["minecraft:block"]) == null ? void 0 : _c.permutations) != null ? _d : []).map((permutation, index) => {
	        var _a2;
	        return [`minecraft:block/permutations/${index}/components`, (_a2 = permutation.components) != null ? _a2 : {}];
	      })
	    ];
	  }
	});

	// What Dash hands every plugin (AllPlugins.ts getPluginContext), on the
	// compiler Rust runs.
	const fileSystem = new FileSystem(0)
	const outputFileSystem = host.separateOutput ? new FileSystem(1) : fileSystem
	const readFile = (filePath) => fileSystem.readFile(filePath)

	// `dash.jsRuntime` (src/Common/JsRuntime.ts), which the built-in plugins
	// run scripts in, and the runtime AllPlugins evaluates extension modules
	// in.
	const jsRuntime = new Runtime(
		[
			["@molang/expressions", expressions],
			["@molang/core", { Molang }],
			["molang", { Molang, ...expressions }],
			["@bridge/compiler", { mode: host.mode }],
		],
		readFile,
	)
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
		builtIn: {
			CustomCommandsPlugin,
			GeneratorScriptsPlugin,
			MolangPlugin,
			CustomEntityComponentPlugin,
			CustomItemComponentPlugin,
			CustomBlockComponentPlugin,
		},
	}
}
