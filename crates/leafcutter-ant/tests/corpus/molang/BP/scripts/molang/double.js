import { expressions } from '@molang/expressions'

const { NumberExpression } = expressions

export default function (expr) {
	if (expr instanceof NumberExpression && expr.eval() === 7) return new NumberExpression(70)
}
