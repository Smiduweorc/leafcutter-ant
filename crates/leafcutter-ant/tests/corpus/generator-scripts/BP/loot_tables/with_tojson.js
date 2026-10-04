class Table {
	toJSON() {
		return { pools: [{ rolls: 1 }], at: 'toJSON' }
	}
}
export default new Table()
