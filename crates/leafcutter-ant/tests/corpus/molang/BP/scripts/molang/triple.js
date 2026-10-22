import { NumberExpression } from '@molang/expressions'
import { Molang } from '@molang/core'
import * as all from 'molang'

const known = new Molang().parse('1 + 2').toString()

export default (expr) => {
	if (expr instanceof NumberExpression && expr.eval() === 7) return new NumberExpression(known === '1+2' && all.Molang === Molang ? 21 : -1)
}
