import { join } from 'pathe'
import { dirname } from 'path-browserify'
import { mode } from '@bridge/compiler'
import helper from './helper'
import data from './data.json'
import { speed } from './data.json'

const health = await Promise.resolve(20)

export default {
	format_version: '1.16.0',
	'minecraft:entity': {
		description: { identifier: helper('zombie'), is_spawnable: true },
		components: {
			'minecraft:health': { value: health, max: health * 1.5 },
			'minecraft:movement': { value: speed },
		},
	},
	meta: { joined: join('a', '../b', 'c'), dir: dirname('x/y/z.json'), mode, data, nothing: undefined, fn() {}, list: [1, undefined, () => 2, NaN, -0, 1e21, 0.1 + 0.2] },
}
