import { arg, operation, ref } from 'aleph-language'
import { numberInput, numberOutput } from '../schemas/common'

export const scaleNumber = operation('calculations.scaleNumber')
  .input(numberInput)
  .output(numberOutput)
  .derive('scaled', arg('number').add(2).multiply(100))
  .return(ref('scaled'))
