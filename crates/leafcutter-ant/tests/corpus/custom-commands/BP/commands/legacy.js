class Legacy {
	static command_name = 'legacy'
	onApply(args) {
		return [`say legacy ${args.length}`]
	}
}
Bridge.register(Legacy)
