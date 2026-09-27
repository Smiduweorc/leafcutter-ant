export default defineCommand(({ name, template, schema }) => {
	name('say_twice')
	schema({ arguments: [{ type: 'string' }] })
	template(([message = 'hi'], { compileCommands }) => [`say ${message}`, ...compileCommands([`say ${message}`])])
})
