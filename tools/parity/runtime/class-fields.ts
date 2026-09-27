export class A {
	static count = 0
	#secret = 1
	constructor(public name: string) {}
	get secret() { return this.#secret }
}
