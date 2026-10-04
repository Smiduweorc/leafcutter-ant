import { useTemplate } from '@bridge/generate'

interface Entity {
	format_version: string
	[key: string]: unknown
}

const template: Entity = await useTemplate('../../templates/base_entity.json')
template['minecraft:entity'] = { description: { identifier: 'custom:skeleton' } }
export default template
