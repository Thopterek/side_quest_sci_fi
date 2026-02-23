/*
 * I lied this is rebuilding of scanner but worse
 * nvm just making String split
*/
public class types {

  public static void main (String[] ag) {
    if (check_args(ag) != 0)
      return;
  }

  public static int check_args(String[] ag) {
    if (2 != ag.length) {
      System.out.println("\u001B[31m" + "Wrong number of arguments, get string and delimiter" + "\u001B[37m");
      return (1);
    }
    if (ag[1].length() != 1) {
      System.out.println("\u001B[31m" + "Delimiter should be 1 character for now" + "\u001B[37m");
      return (1);
    }
    for (int i = 0; i < ag[0].length(); i++) {
      if (ag[1].charAt(0) == ag[0].charAt(1)) {
        break;
      }
      if (i == ag[0].length()) {
        System.out.println("\u001B[31m" + "Delimiter is not in the String to be splitted" + "\u001B[37m");
        return (1);
      }
    }
    System.out.println("To be split -> " +ag[0]);
    System.out.println("Delimiter -> " +ag[1]);
    return (0);
  }

}

