import { useTemplate, createCollection } from '@bridge/generate'
import { join } from 'pathe'

const collection = createCollection()
const template = await useTemplate('./template.json')
for (const name of ['a', 'b']) {
	collection.add(join('out', `${name}.json`), { ...template, name })
}
export default collection
