/*
 * I lied this is rebuilding of scanner but worse
 * nvm just making String split
*/
public class types {

  public static void main (String[] ag) {
    if (check_args(ag) != 0)
      return;
    String[] parts = splitting(ag);
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
      if (ag[1].charAt(0) == ag[0].charAt(i)) {
        break;
      }
      if (i == ag[0].length() - 1) {
        System.out.println("\u001B[31m" + "Delimiter is not in the String to be splitted" + "\u001B[37m");
        return (1);
      }
    }
    System.out.println("\u001B[32m" + "LET'S GOOOOOO" + "\u001B[37m");
    System.out.println("To be split -> " +ag[0]);
    System.out.println("Delimiter -> " +ag[1]);
    return (0);
  }

  public static String[] splitting(String[] ag) {
    int delim_counter = 0;
    for (int i = 0; i < ag[0].length(); i++) {
      if (ag[1].charAt(0) == ag[0].charAt(i)) {
        delim_counter++;
      }
    }
    String parts = new String[delim_counter + 1];
    return (parts);
  }

}

